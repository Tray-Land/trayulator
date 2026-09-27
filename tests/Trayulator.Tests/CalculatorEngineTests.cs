using Trayulator.Core;
using Xunit;

namespace Trayulator.Tests;

public class CalculatorEngineTests
{
    [Theory]
    [InlineData("1+2", "3")]
    [InlineData("2*(3+4)", "14")]
    [InlineData("7/2", "3.5")]
    [InlineData("1/2", "0.5")]
    [InlineData("0.1+0.2", "0.3")]
    [InlineData("1/3", "0.333333333333333")]
    [InlineData("6×7", "42")]
    [InlineData("9÷3", "3")]
    [InlineData("5−8", "-3")]
    [InlineData("2^10", "1024")]
    [InlineData("-2^2", "-4")]
    [InlineData("sqrt(16)", "4")]
    [InlineData("Sqrt(2)*Sqrt(2)", "2")]
    [InlineData("round(2.567, 2)", "2.57")]
    [InlineData("10 % 3", "1")]
    [InlineData("2147483647+1", "2147483648")]
    [InlineData("99999999999*99999999999", "9.9999999998E+21")]
    [InlineData("1e3*2", "2000")]
    [InlineData("pi*2", "6.28318530717959")]
    [InlineData("2π", "6.28318530717959")]
    [InlineData("12+3 =", "15")]
    public async Task Evaluates(string input, string expected)
    {
        CalculationResult result = await CalculatorEngine.EvaluateAsync(input);
        Assert.True(result.Succeeded, result.Text);
        Assert.Equal(expected, result.Text);
    }

    [Fact]
    public async Task AnsIsThePreviousAnswer()
    {
        CalculationResult result = await CalculatorEngine.EvaluateAsync("ans*2", 21L);
        Assert.Equal("42", result.Text);
    }

    [Fact]
    public async Task ResultsCanBeEvaluatedAgain()
    {
        CalculationResult first = await CalculatorEngine.EvaluateAsync("99999999999*99999999999");
        CalculationResult again = await CalculatorEngine.EvaluateAsync(first.Text + "+0");
        Assert.True(again.Succeeded, again.Text);
    }

    [Theory]
    [InlineData("", "Enter an expression")]
    [InlineData("3 +", "Invalid expression")]
    [InlineData("1/0", "Can't divide by zero")]
    [InlineData("5/0.0", "Can't divide by zero")]
    [InlineData("1e308*10", "Result is too large")]
    [InlineData("sqrt(-1)", "Result is undefined")]
    [InlineData("foo(2)", "Unknown function")]
    [InlineData("x+1", "Unknown name in expression")]
    public async Task ExplainsFailures(string input, string expected)
    {
        CalculationResult result = await CalculatorEngine.EvaluateAsync(input);
        Assert.False(result.Succeeded);
        Assert.Equal(expected, result.Text);
    }
}
