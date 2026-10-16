namespace Trayulator.Core;

/// <summary>One evaluated equation: the expression as entered and its formatted result.</summary>
public sealed record HistoryEntry(string Expression, string Result)
{
    /// <summary>What Narrator reads for the entry in the history list.</summary>
    public override string ToString() => $"{Expression} = {Result}";
}

/// <summary>
/// The equations evaluated this session, newest first. Kept in memory only, so it outlives the
/// flyout closing but not the app exiting, like Windows Calculator's history.
/// </summary>
public sealed class CalculationHistory
{
    /// <summary>The most entries kept; the oldest drop off past this.</summary>
    public const int Capacity = 50;

    private readonly List<HistoryEntry> _entries = [];

    /// <summary>Newest first.</summary>
    public IReadOnlyList<HistoryEntry> Entries => _entries;

    /// <summary>Records an equation. Repeating the newest one again doesn't add a duplicate.</summary>
    public void Add(string expression, string result)
    {
        HistoryEntry entry = new(expression.Trim(), result);
        if (_entries.Count > 0 && _entries[0] == entry)
        {
            return;
        }

        _entries.Insert(0, entry);
        if (_entries.Count > Capacity)
        {
            _entries.RemoveAt(_entries.Count - 1);
        }
    }

    public void Clear() => _entries.Clear();
}
