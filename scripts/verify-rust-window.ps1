param([switch]$ReadOnly)

# Exercise only the isolated QA executable recorded by verify-rust-host.mjs.
# This does not use the installed reader or inspect its profile.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$projectRoot = Split-Path -Parent $PSScriptRoot
$qaRoot = [IO.Path]::GetFullPath((Join-Path $projectRoot 'outputs/rust-host-qa'))
$pidFile = Join-Path $qaRoot 'host.pid'
$reportFile = Join-Path $qaRoot 'window-verification.json'
$qaProcessId = [int](Get-Content -LiteralPath $pidFile -Raw).Trim()
$qaProcess = Get-Process -Id $qaProcessId
$processInfo = Get-CimInstance Win32_Process -Filter "ProcessId = $qaProcessId"
$qaExecutable = [IO.Path]::GetFullPath([string]$processInfo.ExecutablePath)
if (-not $qaExecutable.StartsWith($qaRoot + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
    throw 'The recorded process is not an executable inside the isolated QA directory.'
}
$profileMatch = [regex]::Match([string]$processInfo.CommandLine, '--qa-profile\s+(?:"([^"]+)"|(\S+))')
if (-not $profileMatch.Success) { throw 'The recorded process has no explicit QA profile.' }
$profileArgument = if ($profileMatch.Groups[1].Success) { $profileMatch.Groups[1].Value } else { $profileMatch.Groups[2].Value }
if ([IO.Path]::GetFullPath($profileArgument) -ne (Join-Path $qaRoot 'profile')) {
    throw 'The recorded process is not using the expected isolated QA profile.'
}
Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
public static class QuranRustWindowQa {
    [StructLayout(LayoutKind.Sequential)] public struct Point { public int X, Y; }
    [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)] public struct MonitorInfo {
        public uint Size;
        public Rect Monitor, Work;
        public uint Flags;
    }
    [StructLayout(LayoutKind.Sequential)] public struct MinMaxInfo {
        public Point Reserved, MaxSize, MaxPosition, MinTrackSize, MaxTrackSize;
    }
    [StructLayout(LayoutKind.Sequential)] public struct Placement {
        public uint Length, Flags, ShowCommand;
        public Point MinPosition, MaxPosition;
        public Rect NormalPosition;
    }
    [DllImport("user32.dll", SetLastError=true)] public static extern bool IsWindow(IntPtr window);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr window);
    [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr window);
    [DllImport("user32.dll")] public static extern bool IsZoomed(IntPtr window);
    [DllImport("user32.dll", SetLastError=true)] public static extern bool GetWindowRect(IntPtr window, out Rect rect);
    [DllImport("user32.dll", SetLastError=true)] public static extern bool GetClientRect(IntPtr window, out Rect rect);
    [DllImport("user32.dll", SetLastError=true)] public static extern bool ClientToScreen(IntPtr window, ref Point point);
    [DllImport("user32.dll")] public static extern IntPtr MonitorFromWindow(IntPtr window, uint flags);
    [DllImport("user32.dll", CharSet=CharSet.Unicode, SetLastError=true)] public static extern bool GetMonitorInfo(IntPtr monitor, ref MonitorInfo info);
    [DllImport("user32.dll", SetLastError=true)] public static extern bool GetWindowPlacement(IntPtr window, ref Placement placement);
    [DllImport("user32.dll", SetLastError=true)] public static extern bool SetWindowPlacement(IntPtr window, ref Placement placement);
    [DllImport("user32.dll", SetLastError=true)] public static extern bool SetWindowPos(IntPtr window, IntPtr after, int x, int y, int width, int height, uint flags);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr window, int command);
    [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr window);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr window, out uint processId);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr window, StringBuilder text, int maximum);
    [DllImport("user32.dll", SetLastError=true)] public static extern IntPtr SendMessageTimeout(IntPtr window, uint message, UIntPtr wParam, IntPtr lParam, uint flags, uint timeout, out UIntPtr result);
    [DllImport("user32.dll")] public static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);
    private delegate bool EnumerateWindow(IntPtr window, IntPtr parameter);
    [DllImport("user32.dll")] private static extern bool EnumWindows(EnumerateWindow callback, IntPtr parameter);
    public static IntPtr[] WindowsForProcess(uint processId) {
        var windows = new List<IntPtr>();
        EnumWindows((window, parameter) => {
            uint owner;
            GetWindowThreadProcessId(window, out owner);
            if (owner == processId) {
                var name = new StringBuilder(128);
                GetClassName(window, name, name.Capacity);
                if (name.ToString() == "QuranMushafReader.Rust") windows.Add(window);
            }
            return true;
        }, IntPtr.Zero);
        return windows.ToArray();
    }
    public static IntPtr Coordinates(int x, int y) {
        return new IntPtr(unchecked((int)((((uint)y & 0xffff) << 16) | ((uint)x & 0xffff))));
    }
}
'@

# MainWindowHandle omits windows launched with the QA helper's Hidden option.
# Enumerate by PID and the exact host class, never by title or global app name.
[IntPtr[]]$nativeWindows = [QuranRustWindowQa]::WindowsForProcess([uint32]$qaProcessId)
if ($nativeWindows.Count -ne 1) { throw 'The QA process must have exactly one Rust reader window.' }
$targetHandle = $nativeWindows[0]
$initialVisible = [QuranRustWindowQa]::IsWindowVisible($targetHandle)

$checks = [Collections.Generic.List[object]]::new()
$initialPlacement = [QuranRustWindowQa+Placement]::new()
$initialPlacement.Length = [Runtime.InteropServices.Marshal]::SizeOf($initialPlacement)
$savedPlacement = $false
$completed = $false
$oldDpiContext = [QuranRustWindowQa]::SetThreadDpiAwarenessContext([IntPtr](-4))

function Assert-QAWindow {
    if (([int](Get-Content -LiteralPath $pidFile -Raw).Trim()) -ne $qaProcessId) { throw 'The QA process changed during the window test.' }
    $owner = [uint32]0
    [void][QuranRustWindowQa]::GetWindowThreadProcessId($targetHandle, [ref]$owner)
    if (-not [QuranRustWindowQa]::IsWindow($targetHandle) -or $owner -ne $qaProcessId) {
        throw 'The isolated QA window is no longer available.'
    }
}

function Assert-Check([string]$name, [bool]$passed, $details) {
    $checks.Add([ordered]@{ name = $name; passed = $passed; details = $details })
    if (-not $passed) { throw "Native window check failed: $name" }
}

function Read-WindowRect {
    Assert-QAWindow
    $rect = [QuranRustWindowQa+Rect]::new()
    if (-not [QuranRustWindowQa]::GetWindowRect($targetHandle, [ref]$rect)) { throw 'Could not query the QA window geometry.' }
    return $rect
}

function Send-QAMessage([uint32]$message, [IntPtr]$lParam) {
    Assert-QAWindow
    $result = [UIntPtr]::Zero
    if ([QuranRustWindowQa]::SendMessageTimeout($targetHandle, $message, [UIntPtr]::Zero, $lParam, 2, 2000, [ref]$result) -eq [IntPtr]::Zero) {
        throw 'The QA window did not answer the native message within two seconds.'
    }
    return $result.ToUInt64()
}

try {
    Assert-QAWindow
    $className = [Text.StringBuilder]::new(128)
    [void][QuranRustWindowQa]::GetClassName($targetHandle, $className, $className.Capacity)
    Assert-Check 'isolated Rust window class' ($className.ToString() -eq 'QuranMushafReader.Rust') $className.ToString()
    if (-not [QuranRustWindowQa]::GetWindowPlacement($targetHandle, [ref]$initialPlacement)) { throw 'Could not save the QA window placement.' }
    $savedPlacement = $true

    if (-not $ReadOnly) {
        # Show without stealing focus. Tests need a restored window for edge hits.
        [void][QuranRustWindowQa]::ShowWindow($targetHandle, 4)
        Start-Sleep -Milliseconds 150
    }
    Assert-Check 'window restored for hit tests' (-not [QuranRustWindowQa]::IsIconic($targetHandle) -and -not [QuranRustWindowQa]::IsZoomed($targetHandle)) $null
    $rect = Read-WindowRect
    $centerX = [int](($rect.Left + $rect.Right) / 2)
    $centerY = [int](($rect.Top + $rect.Bottom) / 2)
    $hitCases = @(
        @('left edge', ($rect.Left + 2), $centerY, 10),
        @('right edge', ($rect.Right - 2), $centerY, 11),
        @('top edge', $centerX, ($rect.Top + 2), 12),
        @('bottom edge', $centerX, ($rect.Bottom - 2), 15),
        @('top left corner', ($rect.Left + 2), ($rect.Top + 2), 13),
        @('top right corner', ($rect.Right - 2), ($rect.Top + 2), 14),
        @('bottom left corner', ($rect.Left + 2), ($rect.Bottom - 2), 16),
        @('bottom right corner', ($rect.Right - 2), ($rect.Bottom - 2), 17),
        @('reader interior', $centerX, $centerY, 1)
    )
    foreach ($hit in $hitCases) {
        $actual = Send-QAMessage 0x84 ([QuranRustWindowQa]::Coordinates([int]$hit[1], [int]$hit[2]))
        Assert-Check ([string]$hit[0]) ($actual -eq [uint64]$hit[3]) @{ expected = $hit[3]; actual = $actual }
    }

    $limits = [QuranRustWindowQa+MinMaxInfo]::new()
    $limitsMemory = [Runtime.InteropServices.Marshal]::AllocHGlobal([Runtime.InteropServices.Marshal]::SizeOf($limits))
    try {
        [Runtime.InteropServices.Marshal]::StructureToPtr($limits, $limitsMemory, $false)
        [void](Send-QAMessage 0x24 $limitsMemory)
        $limits = [Runtime.InteropServices.Marshal]::PtrToStructure($limitsMemory, [type][QuranRustWindowQa+MinMaxInfo])
    } finally { [Runtime.InteropServices.Marshal]::FreeHGlobal($limitsMemory) }
    $dpi = [Math]::Max(96, [int][QuranRustWindowQa]::GetDpiForWindow($targetHandle))
    $expectedWidth = [int][Math]::Floor(720 * $dpi / 96)
    $expectedHeight = [int][Math]::Floor(480 * $dpi / 96)
    Assert-Check 'minimum track dimensions respect DPI' ($limits.MinTrackSize.X -eq $expectedWidth -and $limits.MinTrackSize.Y -eq $expectedHeight) @{
        dpi = $dpi; width = $limits.MinTrackSize.X; height = $limits.MinTrackSize.Y
    }
    Assert-Check 'maximized bounds have valid size' ($limits.MaxSize.X -gt 0 -and $limits.MaxSize.Y -gt 0) @{
        width = $limits.MaxSize.X; height = $limits.MaxSize.Y
    }

    if (-not $ReadOnly) {
        Assert-QAWindow
        $testWidth = [Math]::Max(1000, $expectedWidth + 80)
        $testHeight = [Math]::Max(760, $expectedHeight + 80)
        if (-not [QuranRustWindowQa]::SetWindowPos($targetHandle, [IntPtr]::Zero, $rect.Left, $rect.Top, $testWidth, $testHeight, 0x14)) { throw 'Could not resize the QA window.' }
        Start-Sleep -Milliseconds 200
        $resized = Read-WindowRect
        Assert-Check 'requested geometry applied' (($resized.Right - $resized.Left) -eq $testWidth -and ($resized.Bottom - $resized.Top) -eq $testHeight) @{
            width = $resized.Right - $resized.Left; height = $resized.Bottom - $resized.Top
        }
        $client = [QuranRustWindowQa+Rect]::new()
        if (-not [QuranRustWindowQa]::GetClientRect($targetHandle, [ref]$client)) { throw 'Could not query the QA client geometry.' }
        Assert-Check 'custom frame retains full client area' (($client.Right - $client.Left) -eq $testWidth -and ($client.Bottom - $client.Top) -eq $testHeight) @{
            width = $client.Right - $client.Left; height = $client.Bottom - $client.Top
        }
        Assert-QAWindow
        [void][QuranRustWindowQa]::ShowWindow($targetHandle, 7)
        Start-Sleep -Milliseconds 200
        Assert-Check 'minimize succeeds' ([QuranRustWindowQa]::IsIconic($targetHandle)) $null
        Assert-QAWindow
        [void][QuranRustWindowQa]::ShowWindow($targetHandle, 4)
        Start-Sleep -Milliseconds 200
        $restored = Read-WindowRect
        Assert-Check 'restore retains resized geometry' (-not [QuranRustWindowQa]::IsIconic($targetHandle) -and ($restored.Right - $restored.Left) -eq $testWidth -and ($restored.Bottom - $restored.Top) -eq $testHeight) $null

        Assert-QAWindow
        [void][QuranRustWindowQa]::ShowWindow($targetHandle, 3)
        Start-Sleep -Milliseconds 200
        $maximized = Read-WindowRect
        $monitorInfo = [QuranRustWindowQa+MonitorInfo]::new()
        $monitorInfo.Size = [Runtime.InteropServices.Marshal]::SizeOf($monitorInfo)
        $monitor = [QuranRustWindowQa]::MonitorFromWindow($targetHandle, 2)
        if ($monitor -eq [IntPtr]::Zero -or -not [QuranRustWindowQa]::GetMonitorInfo($monitor, [ref]$monitorInfo)) { throw 'Could not query the QA monitor work area.' }
        if (-not [QuranRustWindowQa]::GetClientRect($targetHandle, [ref]$client)) { throw 'Could not query the maximized QA client area.' }
        $clientOrigin = [QuranRustWindowQa+Point]::new()
        if (-not [QuranRustWindowQa]::ClientToScreen($targetHandle, [ref]$clientOrigin)) { throw 'Could not query the maximized QA client origin.' }
        Assert-Check 'maximize succeeds' ([QuranRustWindowQa]::IsZoomed($targetHandle)) $null
        Assert-Check 'maximized client fills monitor work area' (($client.Right - $client.Left) -eq ($monitorInfo.Work.Right - $monitorInfo.Work.Left) -and ($client.Bottom - $client.Top) -eq ($monitorInfo.Work.Bottom - $monitorInfo.Work.Top) -and $clientOrigin.X -eq $monitorInfo.Work.Left -and $clientOrigin.Y -eq $monitorInfo.Work.Top) @{
            dpi = [QuranRustWindowQa]::GetDpiForWindow($targetHandle)
            window = @{ left = $maximized.Left; top = $maximized.Top; right = $maximized.Right; bottom = $maximized.Bottom }
            client = @{ x = $clientOrigin.X; y = $clientOrigin.Y; width = $client.Right - $client.Left; height = $client.Bottom - $client.Top }
            work = @{ left = $monitorInfo.Work.Left; top = $monitorInfo.Work.Top; right = $monitorInfo.Work.Right; bottom = $monitorInfo.Work.Bottom }
        }
        Assert-QAWindow
        [void][QuranRustWindowQa]::ShowWindow($targetHandle, 9)
        Start-Sleep -Milliseconds 150
        $afterMaximize = Read-WindowRect
        Assert-Check 'unmaximize restores reader geometry' (-not [QuranRustWindowQa]::IsZoomed($targetHandle) -and ($afterMaximize.Right - $afterMaximize.Left) -eq $testWidth -and ($afterMaximize.Bottom - $afterMaximize.Top) -eq $testHeight) $null
    }
    $completed = $true
} finally {
    if ($savedPlacement -and -not $ReadOnly) {
        try {
            Assert-QAWindow
            if (-not [QuranRustWindowQa]::SetWindowPlacement($targetHandle, [ref]$initialPlacement)) { throw 'The saved QA placement could not be restored.' }
            if (-not $initialVisible) { [void][QuranRustWindowQa]::ShowWindow($targetHandle, 0) }
        } catch {
            $completed = $false
            Write-Warning 'The QA host changed or its placement could not be restored; its replacement was not modified.'
        }
    }
    if ($oldDpiContext -ne [IntPtr]::Zero) { [void][QuranRustWindowQa]::SetThreadDpiAwarenessContext($oldDpiContext) }
    [ordered]@{
        created = [DateTime]::UtcNow.ToString('o')
        processId = $qaProcessId
        executable = $qaExecutable
        readOnly = [bool]$ReadOnly
        passed = ($completed -and $checks.Count -gt 0 -and @($checks | Where-Object { -not $_.passed }).Count -eq 0)
        checks = @($checks.ToArray())
        limitation = 'Direct window-message checks verify native behavior; they do not prove pointer routing through the WebView child.'
    } | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $reportFile -Encoding utf8
}
if (-not $completed) { throw 'Native Rust window QA did not complete.' }
Write-Output "Native Rust window QA passed ($($checks.Count) checks)."
