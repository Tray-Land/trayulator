using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media.Animation;
using Trayulator.Core;
using Trayulator.Services;
using Windows.ApplicationModel.DataTransfer;
using Windows.System;

namespace Trayulator.Views;

/// <summary>
/// The calculator: a free-form expression entry plus a keypad. Enter, "=" or the = key replaces
/// the expression with its result, and the equation moves to the line above, like Windows
/// Calculator. Nothing runs while the flyout is hidden.
/// </summary>
public sealed partial class FlyoutPage : Page, IDisposable
{
    private const string PinGlyph = "";
    private const string PinnedGlyph = "";

    // Static so it outlives the page, which is rebuilt after the flyout's idle close.
    private static readonly CalculationHistory History = new();

    private bool _settingText;
    private bool _isHistoryOpen;
    private bool _justEvaluated;
    private bool _isEvaluating;
    private object? _answer;
    private string? _lastExpression;
    private SettingsPage? _settings;

    public FlyoutPage()
    {
        InitializeComponent();
        TitleText.Text = App.DisplayName;
        PinButton.IsChecked = SettingsService.IsPinned;
        UpdatePinButton();
        ((Storyboard)Resources["HistoryExit"]).Completed += HistoryExit_Completed;
    }

    /// <summary>The flyout opened: always on the calculator, caret at the end of the entry.</summary>
    public void OnShown()
    {
        CloseSettings(animate: false);
        CloseHistory(animate: false);
        FocusEntry();
    }

    /// <summary>Swaps the calculator for the settings page.</summary>
    public void ShowSettings()
    {
        if (_settings is not null)
        {
            return;
        }

        CloseHistory(animate: false);
        _settings = new SettingsPage();
        _settings.BackRequested += (_, _) => GoBack();
        CalculatorView.Visibility = Visibility.Collapsed;
        SettingsHost.Visibility = Visibility.Visible;
        SettingsHost.Content = _settings;
    }

    /// <summary>Returns from settings or history to the calculator; false when already there.</summary>
    public bool GoBack()
    {
        if (!CloseSettings(animate: true) && !CloseHistory(animate: true))
        {
            return false;
        }

        FocusEntry();
        return true;
    }

    private void OpenHistory()
    {
        _isHistoryOpen = true;
        HistoryButton.IsChecked = true;
        RefreshHistory();

        ((Storyboard)Resources["HistoryExit"]).Stop();
        var enter = (Storyboard)Resources["HistoryEnter"];
        ((DoubleAnimation)enter.Children[0]).From = Keypad.ActualHeight;
        HistoryPane.Visibility = Visibility.Visible;
        enter.Begin();

        if (History.Entries.Count > 0)
        {
            HistoryList.Focus(FocusState.Programmatic);
        }
    }

    private bool CloseHistory(bool animate)
    {
        if (!_isHistoryOpen)
        {
            return false;
        }

        _isHistoryOpen = false;
        HistoryButton.IsChecked = false;
        ((Storyboard)Resources["HistoryEnter"]).Stop();
        if (animate)
        {
            var exit = (Storyboard)Resources["HistoryExit"];
            ((DoubleAnimation)exit.Children[0]).To = Keypad.ActualHeight;
            exit.Begin();
        }
        else
        {
            ((Storyboard)Resources["HistoryExit"]).Stop();
            HistoryPane.Visibility = Visibility.Collapsed;
        }

        return true;
    }

    private void HistoryExit_Completed(object? sender, object e)
    {
        // A reopen during the exit animation stops it, but guard in case Completed was already queued.
        if (!_isHistoryOpen)
        {
            HistoryPane.Visibility = Visibility.Collapsed;
        }
    }

    private void RefreshHistory()
    {
        // A snapshot: the list only changes while the pane is closed, or through Clear below.
        HistoryList.ItemsSource = History.Entries.ToArray();
        bool empty = History.Entries.Count == 0;
        HistoryEmptyText.Visibility = empty ? Visibility.Visible : Visibility.Collapsed;
        ClearHistoryButton.IsEnabled = !empty;
    }

    private bool CloseSettings(bool animate)
    {
        if (_settings is null)
        {
            return false;
        }

        // Dropping the page lets it be collected; the next open builds a fresh one.
        _settings = null;
        SettingsHost.Content = null;
        SettingsHost.Visibility = Visibility.Collapsed;
        CalculatorView.Visibility = Visibility.Visible;
        if (animate)
        {
            ((Storyboard)Resources["CalculatorEnter"]).Begin();
        }

        return true;
    }

    private void FocusEntry() => DispatcherQueue.TryEnqueue(() =>
    {
        Entry.Focus(FocusState.Programmatic);
        Entry.Select(Entry.Text.Length, 0);
    });

    public void OnHidden()
    {
    }

    public void Dispose()
    {
    }

    private async void Evaluate()
    {
        string expression = Entry.Text;
        if (_isEvaluating || string.IsNullOrWhiteSpace(expression))
        {
            return;
        }

        _isEvaluating = true;
        try
        {
            CalculationResult result = await CalculatorEngine.EvaluateAsync(expression, _answer);
            if (!result.Succeeded)
            {
                ShowError(result.Text);
                return;
            }

            _answer = result.Value;
            _lastExpression = expression.Trim();
            History.Add(_lastExpression, result.Text);
            SetEntry(result.Text);
            _justEvaluated = true;

            ErrorText.Visibility = Visibility.Collapsed;
            EquationText.Visibility = Visibility.Visible;
            EquationText.Text = $"{_lastExpression} =";
            CopiedBadge.Visibility = SettingsService.CopyResultOnEvaluate && TryCopy(result.Text)
                ? Visibility.Visible
                : Visibility.Collapsed;

            App.Current.SetTrayStatus($"{_lastExpression} = {result.Text}");
        }
        catch (Exception ex)
        {
            // The engine maps the expected failures; anything else still shouldn't take the app down.
            ShowError(ex.Message);
        }
        finally
        {
            _isEvaluating = false;
        }
    }

    private static bool TryCopy(string text)
    {
        try
        {
            DataPackage package = new() { RequestedOperation = DataPackageOperation.Copy };
            package.SetText(text);
            Clipboard.SetContent(package);
            Clipboard.Flush(); // keep the value on the clipboard after the app exits
            return true;
        }
        catch
        {
            // The clipboard can be locked by another app; the result is still on screen.
            return false;
        }
    }

    private void ShowError(string message)
    {
        ErrorText.Text = message;
        ErrorText.Visibility = Visibility.Visible;
        EquationText.Visibility = Visibility.Collapsed;
        CopiedBadge.Visibility = Visibility.Collapsed;
    }

    /// <summary>The user changed the expression: the last result and any error no longer apply.</summary>
    private void OnUserEdit()
    {
        _justEvaluated = false;
        ErrorText.Visibility = Visibility.Collapsed;
        EquationText.Visibility = Visibility.Visible;
        CopiedBadge.Visibility = Visibility.Collapsed;
    }

    /// <summary>Replaces the entry text without it counting as a user edit, caret at <paramref name="caret"/> or the end.</summary>
    private void SetEntry(string text, int? caret = null)
    {
        _settingText = true;
        try
        {
            Entry.Text = text;
        }
        finally
        {
            _settingText = false;
        }

        Entry.Select(Math.Clamp(caret ?? text.Length, 0, text.Length), 0);
    }

    /// <summary>Inserts at the caret, replacing any selection, the way typing would.</summary>
    private void Insert(string value)
    {
        string text = Entry.Text;
        int start = Math.Min(Entry.SelectionStart, text.Length);
        int length = Math.Min(Entry.SelectionLength, text.Length - start);

        // After a result, a number starts a new expression while an operator continues from it.
        if (_justEvaluated && StartsNewExpression(value[0]))
        {
            text = string.Empty;
            start = length = 0;
        }

        OnUserEdit();
        SetEntry(text.Remove(start, length).Insert(start, value), start + value.Length);
    }

    private static bool StartsNewExpression(char c) => char.IsLetterOrDigit(c) || c is '.' or '(' or 'π';

    private static void UpdateFontSize(TextBox entry)
    {
        // Shrink long expressions so more of them fits, like Windows Calculator's display.
        int length = entry.Text.Length;
        entry.FontSize = length switch
        {
            <= 11 => 40,
            <= 15 => 30,
            <= 21 => 22,
            _ => 18,
        };
    }

    private void Entry_BeforeTextChanging(TextBox sender, TextBoxBeforeTextChangingEventArgs args)
    {
        if (_settingText)
        {
            return;
        }

        string newText = args.NewText;
        bool evaluate = newText.Contains('=');
        string clean = newText.Replace("=", string.Empty).Replace("\r", string.Empty).Replace("\n", string.Empty);

        // Typing a number right after a result replaces the result instead of appending to it.
        bool typedAtEnd = newText.Length == sender.Text.Length + 1 && newText.StartsWith(sender.Text, StringComparison.Ordinal);
        if (_justEvaluated && typedAtEnd && StartsNewExpression(newText[^1]))
        {
            clean = newText[^1].ToString();
        }

        if (!evaluate && clean == newText)
        {
            return;
        }

        // Rewriting the text from inside this event isn't allowed; cancel and apply it just after.
        args.Cancel = true;
        DispatcherQueue.TryEnqueue(() =>
        {
            if (clean != sender.Text)
            {
                OnUserEdit();
                SetEntry(clean);
            }

            if (evaluate)
            {
                Evaluate();
            }
        });
    }

    private void Entry_TextChanging(TextBox sender, TextBoxTextChangingEventArgs args)
    {
        UpdateFontSize(sender);
        if (!_settingText)
        {
            OnUserEdit();
        }
    }

    private void Entry_PreviewKeyDown(object sender, KeyRoutedEventArgs e)
    {
        switch (e.Key)
        {
            case VirtualKey.Enter:
                e.Handled = true;
                Evaluate();
                break;

            case VirtualKey.Up when _lastExpression is not null:
                // Recall the last evaluated expression to tweak it.
                e.Handled = true;
                OnUserEdit();
                SetEntry(_lastExpression);
                break;
        }
    }

    private void Key_Click(object sender, RoutedEventArgs e)
    {
        if (sender is FrameworkElement { Tag: string value })
        {
            Insert(value);
        }
    }

    private void Equals_Click(object sender, RoutedEventArgs e) => Evaluate();

    private void Clear_Click(object sender, RoutedEventArgs e)
    {
        OnUserEdit();
        SetEntry(string.Empty);
        EquationText.Text = string.Empty;
    }

    private void Backspace_Click(object sender, RoutedEventArgs e)
    {
        string text = Entry.Text;
        int start = Math.Min(Entry.SelectionStart, text.Length);
        int length = Math.Min(Entry.SelectionLength, text.Length - start);
        if (length == 0)
        {
            if (start == 0)
            {
                return;
            }

            start--;
            length = 1;
        }

        OnUserEdit();
        SetEntry(text.Remove(start, length), start);
    }

    private void PinButton_Click(object sender, RoutedEventArgs e)
    {
        SettingsService.IsPinned = PinButton.IsChecked == true;
        UpdatePinButton();
        Entry.Focus(FocusState.Programmatic);
    }

    private void UpdatePinButton()
    {
        bool pinned = PinButton.IsChecked == true;
        string label = pinned ? "Unpin" : "Pin open";
        PinIcon.Glyph = pinned ? PinnedGlyph : PinGlyph;
        ToolTipService.SetToolTip(PinButton, label);
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(PinButton, label);
    }

    private void SettingsButton_Click(object sender, RoutedEventArgs e) => ShowSettings();

    private void HistoryButton_Click(object sender, RoutedEventArgs e)
    {
        // Ctrl+H invokes the button too, so toggle from our state rather than IsChecked.
        if (_isHistoryOpen)
        {
            GoBack();
        }
        else
        {
            OpenHistory();
        }
    }

    private void HistoryList_ItemClick(object sender, ItemClickEventArgs e)
    {
        if (e.ClickedItem is not HistoryEntry entry)
        {
            return;
        }

        CloseHistory(animate: true);
        OnUserEdit();
        SetEntry(entry.Expression);
        FocusEntry();
    }

    private void ClearHistoryButton_Click(object sender, RoutedEventArgs e)
    {
        History.Clear();
        RefreshHistory();
        HistoryButton.Focus(FocusState.Programmatic);
    }
}
