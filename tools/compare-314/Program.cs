// The 3.1.4 twin: replays the everyday check's edits in dawsu! 3.1.4's own main window, headless (no
// screen) and silent (no audio device opened), then saves through 3.1.4's own Save.
//
//   Snapshot <map copy .osu> <everyday-check results .json> <twin results .json>
//
// The map must be a copy: the twin edits and saves it. It reads the "actions" list the everyday check
// wrote and does each edit at the times dawsu! ended up with (after its snap), so the two apps are
// compared on what they do with the same edit, not on where a mouse landed:
//
//   open      lists the hits 3.1.4 shows on the opened map
//   place     adds a hit on the named lane, through 3.1.4's edit session (the same call its Draw tool makes)
//   move      moves the selected hit by the same amount, through the session
//   volume    types the value into 3.1.4's volume box with "Set all" and applies it (its own clamp applies)
//   delete    presses Delete in the window
//   undo/redo presses Ctrl+Z / Ctrl+Y in the window (3.1.4's default keys in both key styles)
//   add-file  copies the everyday check's silent sample into the copy, as the robot did
//   save      runs 3.1.4's Save and answers each dialog it shows with its Save/Continue button
//
// It writes what 3.1.4 showed after each edit, the dialogs Save showed, and the hits after Save.
// 3.1.4 keeps backups and its project notes under %LOCALAPPDATA%\OsuHitsoundEditor; the files this run
// adds there are listed in the results and removed again, and nothing else there is touched.
using System.Reflection;
using System.Text.Json;
using System.Text.Json.Nodes;
using Avalonia;
using Avalonia.Controls;
using Avalonia.Controls.Primitives;
using Avalonia.Headless;
using Avalonia.Input;
using Avalonia.LogicalTree;
using Avalonia.Threading;
using OsuHitsound.App;
using OsuHitsound.Lanes;

if (args.Length != 3)
{
    Console.Error.WriteLine("usage: Snapshot <map copy .osu> <everyday-check results .json> <twin results .json>");
    return 2;
}
string map = Path.GetFullPath(args[0]);
var robot = JsonNode.Parse(File.ReadAllText(args[1]))!;
var actions = robot["actions"]?.AsArray() ?? throw new InvalidOperationException("the everyday check results list no actions");
string outPath = Path.GetFullPath(args[2]);

string dataRoot = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "OsuHitsoundEditor");
var dataBefore = Files(dataRoot);
// 3.1.4's status-line message (MainWindow.axaml.cs:47).
PropertyInfo Flash = typeof(MainWindow).GetProperty("flash", BindingFlags.NonPublic | BindingFlags.Instance)!;

UiTheme.Current = UiTheme.Load(Path.Combine(AppContext.BaseDirectory, "ui-theme.json"));
AppBuilder.Configure<App>().UseSkia().UseHeadless(new AvaloniaHeadlessPlatformOptions { UseHeadlessDrawing = false }).WithInterFont().SetupWithoutStarting();

var steps = new JsonArray();
var dialogs = new JsonArray();
bool saved = false;
string? error = null;
MainWindow? w = null;
EditHit? selected = null;
try
{
    // startAudio: false opens no audio device; usePreferences: false reads and writes no 3.1.4 settings.
    // Save decodes samples with BASS, which 3.1.4's audio engine would have started. Device 0 is BASS's
    // "no sound" device: it decodes and plays nothing, as 3.1.4's own SilentSamples.cs:38 starts it.
    if (!ManagedBass.Bass.Init(0) && ManagedBass.Bass.LastError != ManagedBass.Errors.Already)
        throw new InvalidOperationException($"BASS could not start for decoding ({ManagedBass.Bass.LastError})");
    w = new MainWindow(map, startAudio: false, showInspector: false, usePreferences: false) { Width = 1600, Height = 900 };
    w.Show();
    Wait(() => w.Loading.IsCompleted, "the map to open");
    Pump();
    var lanes = w.ArrangeArea.Model?.Session.Lanes ?? throw new InvalidOperationException("3.1.4 opened no lanes");

    foreach (var node in actions)
    {
        var a = node!.AsObject();
        string kind = (string)a["do"]!;
        var s = w.ArrangeArea.Model!.Session;
        var result = new JsonObject { ["step"] = a["step"]?.DeepClone(), ["do"] = kind };
        switch (kind)
        {
            case "place":
            {
                string name = (string)a["lane"]!;
                double time = (double)a["seen"]!;
                var lane = s.Lanes.FirstOrDefault(l => Strip(l.Name) == name)
                    ?? throw new InvalidOperationException($"3.1.4 has no lane {name}; it has {string.Join(", ", s.Lanes.Select(l => Strip(l.Name)))}");
                var hit = s.HitAt(lane, time);
                s.Add([hit]);
                Pump();
                selected = s.State.Hits.FirstOrDefault(h => h.Lane == lane && Math.Abs(h.Time - time) < 1);
                if (selected is not null) w.ArrangeArea.Edits.Select([selected]);
                Pump();
                result["seen"] = selected is null ? null : Math.Round(selected.Time);
                result["anchored"] = selected?.Anchor is not null;
                break;
            }
            case "move":
            {
                if (selected is null) { result["note"] = "nothing selected"; break; }
                var moved = s.Move([selected], (double)a["by"]!, h => h.Lane);
                selected = moved.FirstOrDefault();
                if (selected is not null) w.ArrangeArea.Edits.Select([selected]);
                Pump();
                result["seen"] = selected is null ? null : Math.Round(selected.Time);
                break;
            }
            case "volume":
            {
                int value = (int)a["value"]!;
                Control("VolumeModeBox").SetValue(SelectingItemsControl.SelectedIndexProperty, 1); // "Set all"
                ((TextBox)Control("VolumeValueBox")).Text = value.ToString();
                Flash.SetValue(w, null);
                typeof(MainWindow).GetMethod("ApplyVolumeValue", BindingFlags.NonPublic | BindingFlags.Instance)!.Invoke(w, null);
                Pump();
                s = w.ArrangeArea.Model!.Session;
                var now = selected is null ? null : s.State.Hits.FirstOrDefault(h => h.Lane == selected.Lane && Math.Abs(h.Time - selected.Time) < 1);
                result["value"] = value;
                result["seen"] = now is null ? null : s.VolumeOf(now);
                // 3.1.4 says in its status line why a value did not apply (for example a hit away from every object).
                if (Flash.GetValue(w) is string { Length: > 0 } said) result["status"] = said;
                break;
            }
            case "open": result["seen"] = Named(w); break;
            case "delete": Press(Key.Delete); result["seen"] = Named(w); break;
            case "undo": Press(Key.Z, RawInputModifiers.Control); result["seen"] = Named(w); break;
            case "redo": Press(Key.Y, RawInputModifiers.Control); result["seen"] = Named(w); break;
            case "add-file":
            {
                string file = (string)a["file"]!;
                string from = Path.Combine(Path.GetDirectoryName((string)robot["mapCopy"]!)!, file);
                if (File.Exists(from)) File.Copy(from, Path.Combine(Path.GetDirectoryName(map)!, file), overwrite: false);
                else result["note"] = $"the everyday check's {file} is gone; nothing copied";
                break;
            }
            case "save":
            {
                Flash.SetValue(w, null);
                var task = (Task<bool>)typeof(MainWindow).GetMethod("Save", BindingFlags.NonPublic | BindingFlags.Instance)!.Invoke(w, null)!;
                var until = DateTime.UtcNow.AddSeconds(60);
                while (!task.IsCompleted && DateTime.UtcNow < until)
                {
                    Pump();
                    if (w.OwnedWindows.LastOrDefault(x => x.IsVisible) is { } dialog)
                    {
                        var buttons = dialog.GetLogicalDescendants().OfType<Button>().Where(b => b.IsEffectivelyVisible && b.IsEffectivelyEnabled).ToList();
                        var labels = buttons.Select(b => b.Content?.ToString() ?? "").ToList();
                        var pick = buttons.FirstOrDefault(b => b.Content?.ToString() is "Save" or "Continue to Save" or "Continue" or "OK" or "Yes" or "Done");
                        dialogs.Add(new JsonObject { ["title"] = dialog.Title, ["buttons"] = new JsonArray([.. labels.Select(l => (JsonNode)l)]), ["chose"] = pick?.Content?.ToString() });
                        if (pick is null) { dialog.Close(); throw new InvalidOperationException($"Save showed \"{dialog.Title}\" with no Save button: {string.Join(", ", labels)}"); }
                        pick.RaiseEvent(new Avalonia.Interactivity.RoutedEventArgs(Button.ClickEvent));
                    }
                    Thread.Sleep(10);
                }
                if (!task.IsCompleted) throw new TimeoutException("3.1.4's Save did not finish in 60 s");
                saved = task.Result;
                Wait(() => w.Loading.IsCompleted, "the saved map to reopen");
                result["seen"] = Named(w);
                result["saved"] = saved;
                // 3.1.4 says in its status line why a save stopped (MapWrites.cs:63-64, 93-95).
                if (Flash.GetValue(w) is string { Length: > 0 } said) result["status"] = said;
                break;
            }
            default: result["note"] = "not replayed"; break;
        }
        steps.Add(result);
    }
}
catch (Exception ex)
{
    error = (ex is TargetInvocationException { InnerException: { } inner } ? inner : ex).Message;
}
finally
{
    w?.Close();
    Pump();
}

// Remove what this run added to 3.1.4's data folder (backups and notes for this copy), nothing else.
var added = Files(dataRoot).Where(f => !dataBefore.Contains(f)).ToList();
foreach (var file in added)
    try { File.Delete(file); } catch (IOException) { } catch (UnauthorizedAccessException) { }

var output = new JsonObject
{
    ["map"] = map,
    ["saved"] = saved,
    ["error"] = error,
    ["steps"] = steps,
    ["dialogs"] = dialogs,
    ["removed_from_3_1_4_data"] = new JsonArray([.. added.Select(f => (JsonNode)f)]),
};
File.WriteAllText(outPath, output.ToJsonString(new JsonSerializerOptions { WriteIndented = true }) + "\n");
Console.WriteLine(error is null ? $"3.1.4 twin: {steps.Count} edits replayed, saved: {saved}" : $"3.1.4 twin stopped: {error}");
return error is null && saved ? 0 : 1;

Control Control(string name) => w!.FindControl<Control>(name) ?? throw new InvalidOperationException($"3.1.4's window has no {name}");
void Press(Key key, RawInputModifiers mods = RawInputModifiers.None) { w!.KeyPress(key, mods); w.KeyRelease(key, mods); Pump(); }
void Pump() { for (int i = 0; i < 8; i++) Dispatcher.UIThread.RunJobs(); AvaloniaHeadlessPlatform.ForceRenderTimerTick(); }
void Wait(Func<bool> done, string what)
{
    var until = DateTime.UtcNow.AddSeconds(30);
    while (!done() && DateTime.UtcNow < until) { Pump(); Thread.Sleep(5); }
    Pump();
    if (!done()) throw new TimeoutException($"timed out waiting for {what}");
}
// Hits as the everyday check names them: "3500 ms normal-hitwhistle", sorted.
static JsonArray Named(MainWindow window)
{
    var session = window.ArrangeArea.Model!.Session;
    return new JsonArray([.. session.State.Hits.Select(h => $"{Math.Round(h.Time)} ms {Strip(h.Lane.Name)}").Order(StringComparer.Ordinal).Select(n => (JsonNode)n)]);
}
static string Strip(string name) => Path.GetExtension(name).ToLowerInvariant() is ".wav" or ".ogg" or ".mp3" ? Path.GetFileNameWithoutExtension(name) : name;
static HashSet<string> Files(string root) => Directory.Exists(root) ? Directory.EnumerateFiles(root, "*", SearchOption.AllDirectories).ToHashSet(StringComparer.OrdinalIgnoreCase) : [];
