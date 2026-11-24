# The robot mapper's hands outside the web page: the editor's native file pickers and the Explorer
# windows it opens. tools/robot/native.mjs starts this once and sends one command per line on
# standard input; each answer is one line of JSON on standard output.
#
#   dialogs <pid>              the editor's open file or folder pickers: [{ hwnd, title }]
#   cancel <pid>               close every one of them as Cancel would
#   choose <pid> <path>        type <path> into the open picker's name box and press its OK button
#   explorers                  every open Explorer folder window: [{ hwnd, title }]
#   close <hwnd>               close that window as its X would
#   closemain <pid>            press the X of the editor's main window (its title ends in "dawsu!")
#
# Windows only. It only reads windows and sends them the messages a click or a typed name sends.
$ErrorActionPreference = "Stop"
# Paths and window titles from any mapset (Japanese names, accents) pass through unchanged.
[Console]::InputEncoding = [System.Text.Encoding]::UTF8
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
Add-Type @"
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
public static class RobotWindows {
  public delegate bool EnumProc(IntPtr hwnd, IntPtr lparam);
  [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc callback, IntPtr lparam);
  [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr parent, EnumProc callback, IntPtr lparam);
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
  [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr hwnd);
  [DllImport("user32.dll")] static extern bool IsWindowEnabled(IntPtr hwnd);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetClassName(IntPtr hwnd, StringBuilder text, int size);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetWindowText(IntPtr hwnd, StringBuilder text, int size);
  [DllImport("user32.dll")] static extern int GetDlgCtrlID(IntPtr hwnd);
  [DllImport("user32.dll")] static extern IntPtr GetParent(IntPtr hwnd);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern IntPtr SendMessage(IntPtr hwnd, uint message, IntPtr wparam, string lparam);
  [DllImport("user32.dll")] static extern IntPtr SendMessage(IntPtr hwnd, uint message, IntPtr wparam, IntPtr lparam);
  [DllImport("user32.dll")] static extern bool PostMessage(IntPtr hwnd, uint message, IntPtr wparam, IntPtr lparam);
  [DllImport("user32.dll")] static extern IntPtr GetDlgItem(IntPtr hwnd, int id);
  const uint WM_SETTEXT = 0x000C, WM_CLOSE = 0x0010, BM_CLICK = 0x00F5;

  public static string ClassOf(IntPtr hwnd) { var s = new StringBuilder(256); GetClassName(hwnd, s, 256); return s.ToString(); }
  public static string TitleOf(IntPtr hwnd) { var s = new StringBuilder(512); GetWindowText(hwnd, s, 512); return s.ToString(); }

  public static List<IntPtr> TopLevel(string className, int pid) {
    var found = new List<IntPtr>();
    EnumWindows((hwnd, l) => {
      if (!IsWindowVisible(hwnd) || ClassOf(hwnd) != className) return true;
      uint owner; GetWindowThreadProcessId(hwnd, out owner);
      if (pid <= 0 || owner == (uint)pid) found.Add(hwnd);
      return true;
    }, IntPtr.Zero);
    return found;
  }

  public static List<IntPtr> Windows(int pid) {
    var found = new List<IntPtr>();
    EnumWindows((hwnd, l) => {
      if (!IsWindowVisible(hwnd)) return true;
      uint owner; GetWindowThreadProcessId(hwnd, out owner);
      if (owner == (uint)pid && ClassOf(hwnd) != "#32770" && TitleOf(hwnd).Length > 0) found.Add(hwnd);
      return true;
    }, IntPtr.Zero);
    return found;
  }

  public static void Close(IntPtr hwnd) { PostMessage(hwnd, WM_CLOSE, IntPtr.Zero, IntPtr.Zero); }

  [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern IntPtr SendMessage(IntPtr hwnd, uint message, IntPtr wparam, StringBuilder lparam);
  const uint WM_GETTEXT = 0x000D;

  // The picker's name box: only the Edit inside the control with id 1148 (the file name box of a
  // file picker) or 1152 (the "Folder:" box of a folder picker). Any other box (search, address) is
  // never typed into, so a wrong folder is never picked.
  public static string Choose(IntPtr dialog, string path) {
    IntPtr edit = IntPtr.Zero;
    var seen = new List<string>();
    EnumChildWindows(dialog, (hwnd, l) => {
      if (ClassOf(hwnd) != "Edit") return true;
      var ids = new List<string>();
      for (IntPtr up = hwnd; up != IntPtr.Zero && up != dialog; up = GetParent(up)) ids.Add(GetDlgCtrlID(up).ToString());
      seen.Add("Edit[" + string.Join(">", ids) + (IsWindowVisible(hwnd) ? "" : " hidden") + (IsWindowEnabled(hwnd) ? "" : " disabled") + "]");
      if (!IsWindowVisible(hwnd) || !IsWindowEnabled(hwnd)) return true;
      for (IntPtr up = hwnd; up != IntPtr.Zero && up != dialog; up = GetParent(up)) {
        int id = GetDlgCtrlID(up);
        if (id == 1148 || id == 1152) { edit = hwnd; return false; }
      }
      return true;
    }, IntPtr.Zero);
    if (edit == IntPtr.Zero) return "the picker has no name box (control 1148 or 1152); its text boxes: " + (seen.Count == 0 ? "none" : string.Join(", ", seen));
    SendMessage(edit, WM_SETTEXT, IntPtr.Zero, path);
    var typed = new StringBuilder(1024);
    SendMessage(edit, WM_GETTEXT, (IntPtr)1024, typed);
    if (typed.ToString() != path) return "the name box holds \"" + typed + "\" after typing";
    IntPtr ok = GetDlgItem(dialog, 1);
    if (ok == IntPtr.Zero) return "the picker has no OK button (control 1)";
    SendMessage(ok, BM_CLICK, IntPtr.Zero, IntPtr.Zero);
    return "";
  }
}
"@

function Listed($handles) {
  ,@($handles | ForEach-Object { @{ hwnd = [int64]$_; title = [RobotWindows]::TitleOf($_) } })
}

[Console]::Out.WriteLine('{"ready":true}')
[Console]::Out.Flush()
while ($null -ne ($line = [Console]::In.ReadLine())) {
  $parts = $line.Split(" ", 3)
  try {
    switch ($parts[0]) {
      "dialogs" { $answer = @{ ok = $true; windows = (Listed ([RobotWindows]::TopLevel("#32770", [int]$parts[1]))) } }
      "cancel" {
        $windows = [RobotWindows]::TopLevel("#32770", [int]$parts[1])
        foreach ($hwnd in $windows) { [RobotWindows]::Close($hwnd) }
        $answer = @{ ok = $true; closed = $windows.Count }
      }
      "choose" {
        $windows = [RobotWindows]::TopLevel("#32770", [int]$parts[1])
        if ($windows.Count -eq 0) { $answer = @{ ok = $false; error = "no picker is open" } }
        else {
          $problem = [RobotWindows]::Choose($windows[0], $parts[2])
          $answer = @{ ok = ($problem -eq ""); error = $problem; title = [RobotWindows]::TitleOf($windows[0]) }
        }
      }
      "explorers" { $answer = @{ ok = $true; windows = (Listed ([RobotWindows]::TopLevel("CabinetWClass", 0))) } }
      "close" { [RobotWindows]::Close([IntPtr][int64]$parts[1]); $answer = @{ ok = $true } }
      "closemain" {
        $windows = [RobotWindows]::Windows([int]$parts[1])
        $main = $windows | Where-Object { [RobotWindows]::TitleOf($_) -like "*dawsu!" } | Select-Object -First 1
        if ($null -eq $main) { $answer = @{ ok = $false; error = "no window of the editor is open" } }
        else { [RobotWindows]::Close($main); $answer = @{ ok = $true; title = [RobotWindows]::TitleOf($main) } }
      }
      default { $answer = @{ ok = $false; error = "unknown command $($parts[0])" } }
    }
  } catch {
    $answer = @{ ok = $false; error = $_.Exception.Message }
  }
  [Console]::Out.WriteLine(($answer | ConvertTo-Json -Compress -Depth 4))
  [Console]::Out.Flush()
}
