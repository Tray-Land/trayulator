using Trayulator.Core;
using Xunit;

namespace Trayulator.Tests;

public class CalculationHistoryTests
{
    [Fact]
    public void Add_NewEntries_AreNewestFirst()
    {
        CalculationHistory history = new();

        history.Add("1+1", "2");
        history.Add("2*3", "6");

        Assert.Equal([new HistoryEntry("2*3", "6"), new HistoryEntry("1+1", "2")], history.Entries);
    }

    [Fact]
    public void Add_TrimsExpression()
    {
        CalculationHistory history = new();

        history.Add("  1+1 ", "2");

        Assert.Equal("1+1", history.Entries[0].Expression);
    }

    [Fact]
    public void Add_SameAsNewest_IsNotDuplicated()
    {
        CalculationHistory history = new();

        history.Add("1+1", "2");
        history.Add("1+1", "2");

        Assert.Single(history.Entries);
    }

    [Fact]
    public void Add_SameAsOlder_IsAddedAgain()
    {
        CalculationHistory history = new();

        history.Add("1+1", "2");
        history.Add("2*3", "6");
        history.Add("1+1", "2");

        Assert.Equal(3, history.Entries.Count);
        Assert.Equal("1+1", history.Entries[0].Expression);
    }

    [Fact]
    public void Add_PastCapacity_DropsOldest()
    {
        CalculationHistory history = new();

        for (int i = 0; i <= CalculationHistory.Capacity; i++)
        {
            history.Add($"{i}+0", $"{i}");
        }

        Assert.Equal(CalculationHistory.Capacity, history.Entries.Count);
        Assert.Equal($"{CalculationHistory.Capacity}+0", history.Entries[0].Expression);
        Assert.Equal("1+0", history.Entries[^1].Expression);
    }

    [Fact]
    public void HistoryEntry_ToString_ReadsAsEquation()
    {
        Assert.Equal("2*3 = 6", new HistoryEntry("2*3", "6").ToString());
    }

    [Fact]
    public void Clear_RemovesEverything()
    {
        CalculationHistory history = new();
        history.Add("1+1", "2");

        history.Clear();

        Assert.Empty(history.Entries);
    }
}
