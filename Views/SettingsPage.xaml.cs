using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Trayulator.Services;
using Windows.ApplicationModel;

namespace Trayulator.Views;

/// <summary>
/// Settings, shown inside the flyout in place of the calculator. Built fresh each time it opens,
/// so it always reflects the current startup state. Changes apply immediately.
/// </summary>
public sealed partial class SettingsPage : Page
{
    private bool _loading = true;

    public SettingsPage()
    {
        InitializeComponent();
        VersionText.Text = $"{App.DisplayName} {GetVersion()}";
        CopyResultToggle.IsOn = SettingsService.CopyResultOnEvaluate;
        _ = LoadStartupStateAsync();
        _loading = false;

        // Focus lands on the first focusable element (the back button) when the calculator
        // collapses; move it once the page is in the tree.
        Loaded += (_, _) => FocusFirst();
    }

    /// <summary>The user asked to go back to the calculator (back button).</summary>
    public event EventHandler? BackRequested;

    /// <summary>Focuses the first setting, not the back button, whose tooltip would pop up on focus.</summary>
    private void FocusFirst() => CopyResultToggle.Focus(FocusState.Programmatic);

    private static string GetVersion()
    {
        try
        {
            PackageVersion v = Package.Current.Id.Version;
            return $"{v.Major}.{v.Minor}.{v.Build}";
        }
        catch
        {
            return "(unpackaged)";
        }
    }

    private async Task LoadStartupStateAsync()
    {
        ShowStartupState(await StartupService.GetStateAsync());
    }

    private void ShowStartupState(StartupTaskState? state)
    {
        _loading = true;
        StartupToggle.IsOn = state is StartupTaskState.Enabled or StartupTaskState.EnabledByPolicy;

        // The user (Task Manager, Settings > Apps > Startup) or policy has the final say; the app
        // can't override it, so say where to change it instead of offering a dead toggle.
        StartupToggle.IsEnabled = state is StartupTaskState.Enabled or StartupTaskState.Disabled;
        StartupDescription.Text = state switch
        {
            StartupTaskState.DisabledByUser => "Turned off in Settings > Apps > Startup. Turn it on there.",
            StartupTaskState.DisabledByPolicy or StartupTaskState.EnabledByPolicy => "Managed by your organization.",
            null => "Only available when the app is installed.",
            _ => "Keep the tray icon ready after you sign in.",
        };
        _loading = false;
    }

    private void BackButton_Click(object sender, RoutedEventArgs e) => BackRequested?.Invoke(this, EventArgs.Empty);

    private void CopyResultToggle_Toggled(object sender, RoutedEventArgs e)
    {
        if (!_loading)
        {
            SettingsService.CopyResultOnEvaluate = CopyResultToggle.IsOn;
        }
    }

    private async void StartupToggle_Toggled(object sender, RoutedEventArgs e)
    {
        if (_loading)
        {
            return;
        }

        ShowStartupState(await StartupService.SetEnabledAsync(StartupToggle.IsOn));
    }
}
