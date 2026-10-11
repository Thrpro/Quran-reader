param([ValidateSet('left','right','focus')][string]$Action='focus')
$ErrorActionPreference='Stop'
$qaRoot=[IO.Path]::GetFullPath((Join-Path (Split-Path -Parent $PSScriptRoot) 'outputs/focus-qa'))
$qaProcessId=[int](Get-Content -LiteralPath (Join-Path $qaRoot 'host.pid') -Raw).Trim()
$processInfo=Get-CimInstance Win32_Process -Filter "ProcessId = $qaProcessId"
if (-not $processInfo -or [IO.Path]::GetFullPath($processInfo.ExecutablePath) -ne (Join-Path $qaRoot 'qa.exe')) { throw 'Focus check requires the isolated QA executable.' }
if (-not $processInfo.CommandLine.Contains((Join-Path $qaRoot 'profile'))) { throw 'Focus check requires the isolated QA profile.' }
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Text;
public static class FocusQa {
  [StructLayout(LayoutKind.Sequential)] public struct GuiInfo {
    public uint Size, Flags; public IntPtr Active, Focus, Capture, MenuOwner, MoveSize, Caret;
    public int Left, Top, Right, Bottom;
  }
  private delegate bool EnumWindow(IntPtr window, IntPtr parameter);
  [DllImport("user32.dll")] private static extern bool EnumWindows(EnumWindow callback, IntPtr parameter);
  [DllImport("user32.dll",CharSet=CharSet.Unicode)] private static extern int GetClassName(IntPtr window, StringBuilder value, int length);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr window, out uint processId);
  [DllImport("kernel32.dll")] public static extern uint GetCurrentThreadId();
  [DllImport("user32.dll")] public static extern bool AttachThreadInput(uint from, uint to, bool attach);
  [DllImport("user32.dll")] public static extern IntPtr SetFocus(IntPtr window);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr window);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr window, int command);
  [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr window);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr window);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern bool GetGUIThreadInfo(uint threadId, ref GuiInfo info);
  [DllImport("user32.dll")] public static extern bool IsChild(IntPtr parent, IntPtr child);
  [DllImport("user32.dll",SetLastError=true)] public static extern bool PostMessage(IntPtr window, uint message, UIntPtr wParam, IntPtr lParam);
  public static IntPtr Host(uint id) {
    IntPtr result=IntPtr.Zero; EnumWindows((window, unused)=>{uint owner;GetWindowThreadProcessId(window,out owner);
      var name=new StringBuilder(128);GetClassName(window,name,name.Capacity);
      if(owner==id && name.ToString()=="QuranMushafReader.Rust") result=window;return true;},IntPtr.Zero);return result;
  }
}
'@
$window=[FocusQa]::Host([uint32]$qaProcessId)
if ($window -eq [IntPtr]::Zero) { throw 'QA host window missing.' }
[uint32]$ownerId=0
$threadId=[FocusQa]::GetWindowThreadProcessId($window,[ref]$ownerId)
$currentThread=[FocusQa]::GetCurrentThreadId()
if (-not [FocusQa]::AttachThreadInput($currentThread,$threadId,$true)) { throw 'Cannot attach QA focus input.' }
try {
  if ([FocusQa]::IsIconic($window) -or -not [FocusQa]::IsWindowVisible($window)) { [void][FocusQa]::ShowWindow($window, 9) }
  [void][FocusQa]::SetFocus($window)
  $info=[FocusQa+GuiInfo]::new();$info.Size=[Runtime.InteropServices.Marshal]::SizeOf($info)
  if (-not [FocusQa]::GetGUIThreadInfo($threadId,[ref]$info)) { throw 'Cannot inspect QA focus.' }
  if ($info.Focus -eq $window -or $info.Focus -eq [IntPtr]::Zero) { throw 'Native focus remained on the parent instead of returning to WebView2.' }
  if (-not [FocusQa]::IsChild($window,$info.Focus)) { throw 'Focused window is outside the owned QA reader.' }
  if ($Action -ne 'focus') {
    $key=if($Action -eq 'left'){0x25}else{0x27}
    [void][FocusQa]::PostMessage($info.Focus,0x100,[UIntPtr]::new([uint64]$key),[IntPtr]1)
    [void][FocusQa]::PostMessage($info.Focus,0x101,[UIntPtr]::new([uint64]$key),[IntPtr](-1073741823))
  }
  [PSCustomObject]@{Parent=$window.ToInt64();Focus=$info.Focus.ToInt64();Action=$Action;Restored=$true}|ConvertTo-Json -Compress
} finally { [void][FocusQa]::AttachThreadInput($currentThread,$threadId,$false) }
