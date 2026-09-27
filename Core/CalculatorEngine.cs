using System.Globalization;
using System.Text.RegularExpressions;
using NCalc;
using NCalc.Exceptions;

namespace Trayulator.Core;

/// <summary>The outcome of one evaluation: the display text of the result, or why there isn't one.</summary>
public readonly record struct CalculationResult(bool Succeeded, string Text, object? Value)
{
    public static CalculationResult Success(string text, object? value) => new(true, text, value);

    public static CalculationResult Failure(string message) => new(false, message, null);
}

/// <summary>
/// Turns typed or keypad input into a number with NCalc. Pure logic with no WinRT dependency, so
/// the test project compiles this file directly.
/// </summary>
/// <remarks>
/// Input and output both use the invariant culture ('.' decimal point, ',' between function
/// arguments), so a result can be edited and evaluated again as-is.
/// </remarks>
public static partial class CalculatorEngine
{
    /// <summary>Display precision for doubles; hides binary noise such as 0.1 + 0.2 = 0.30000000000000004.</summary>
    private const string DoubleFormat = "G15";

    private const ExpressionOptions Options =
        ExpressionOptions.IgnoreCaseAtBuiltInFunctions | ExpressionOptions.OverflowProtection | ExpressionOptions.LongAsDefault;

    /// <summary>Normalizes keypad symbols and calculator habits into NCalc syntax.</summary>
    public static string Normalize(string input)
    {
        string text = ImplicitPi().Replace(input, "*π")
            .Replace('×', '*')
            .Replace('÷', '/')
            .Replace('−', '-')
            .Replace("π", "pi", StringComparison.Ordinal)
            .Replace("^", "**", StringComparison.Ordinal) // NCalc's ^ is bitwise XOR
            .Trim();

        return text.TrimEnd('=').TrimEnd();
    }

    /// <summary>Evaluates <paramref name="input"/>; <paramref name="previousAnswer"/> is available as <c>ans</c>.</summary>
    public static async Task<CalculationResult> EvaluateAsync(string input, object? previousAnswer = null, CancellationToken cancellationToken = default)
    {
        string expression = Normalize(input);
        if (expression.Length == 0)
        {
            return CalculationResult.Failure("Enter an expression");
        }

        try
        {
            return Format(await EvaluateCoreAsync(expression, previousAnswer, cancellationToken));
        }
        catch (OverflowException)
        {
            // Integer math overflowed (for example 99999999999 * 99999999999). Retry with every
            // integer literal as a double, which trades exactness for range like a real calculator.
            try
            {
                return Format(await EvaluateCoreAsync(IntegerLiteral().Replace(expression, "$0.0"), previousAnswer, cancellationToken));
            }
            catch (OverflowException)
            {
                return CalculationResult.Failure(DivisionByZero().IsMatch(expression) ? "Can't divide by zero" : "Result is too large");
            }
            catch (Exception ex) when (ex is NCalcException or ArgumentException or InvalidCastException or FormatException)
            {
                return CalculationResult.Failure(Describe(ex));
            }
        }
        catch (DivideByZeroException)
        {
            return CalculationResult.Failure("Can't divide by zero");
        }
        catch (Exception ex) when (ex is NCalcException or ArgumentException or InvalidCastException or FormatException)
        {
            return CalculationResult.Failure(Describe(ex));
        }
    }

    /// <summary>Formats an evaluated value as text that can be evaluated again.</summary>
    public static CalculationResult Format(object? value) => value switch
    {
        double d when double.IsNaN(d) => CalculationResult.Failure("Result is undefined"),
        double d when double.IsInfinity(d) => CalculationResult.Failure("Result is too large"),
        double d => CalculationResult.Success(FormatDouble(d), d),
        float f => Format((double)f),
        decimal m => CalculationResult.Success(m.ToString(CultureInfo.InvariantCulture), m),
        IFormattable number => CalculationResult.Success(number.ToString(null, CultureInfo.InvariantCulture), value),
        bool b => CalculationResult.Success(b ? "true" : "false", b),
        null => CalculationResult.Failure("No result"),
        _ => CalculationResult.Failure("Not a number"),
    };

    private static async Task<object?> EvaluateCoreAsync(string expression, object? previousAnswer, CancellationToken cancellationToken)
    {
        Dictionary<string, object?> parameters = new(StringComparer.OrdinalIgnoreCase)
        {
            ["pi"] = Math.PI,
            ["e"] = Math.E,
            ["ans"] = previousAnswer ?? 0,
        };

        ExpressionContext context = new(parameters, null!, null!, null!, null!);
        Expression parsed = new(expression, ExpressionConfiguration.FromOptions(Options), context, CultureInfo.InvariantCulture, null!);
        return await parsed.EvaluateAsync(cancellationToken);
    }

    private static string FormatDouble(double value)
    {
        // Round-trip through G15 so the stored digits match what's shown; -0 reads as 0.
        string text = value.ToString(DoubleFormat, CultureInfo.InvariantCulture);
        return text == "-0" ? "0" : text;
    }

    private static string Describe(Exception ex) => ex switch
    {
        NCalcParameterNotDefinedException => "Unknown name in expression",
        NCalcFunctionNotFoundException => "Unknown function",
        NCalcParserException => "Invalid expression",
        _ => "Can't evaluate that",
    };

    /// <summary>Integer literals: digits not touching a '.', an exponent, or an identifier.</summary>
    [GeneratedRegex(@"(?<![\w.])\d+(?![\w.])")]
    private static partial Regex IntegerLiteral();

    /// <summary>π written straight after a number or bracket, as in 2π, which means 2×π.</summary>
    [GeneratedRegex(@"(?<=[\d)])π")]
    private static partial Regex ImplicitPi();

    /// <summary>A literal zero divisor, the usual reason a quotient overflows.</summary>
    [GeneratedRegex(@"/\s*0*\.?0+(?![\w.])")]
    private static partial Regex DivisionByZero();
}
