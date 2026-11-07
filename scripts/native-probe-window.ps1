param([int]$HostProcessId,[ValidateSet('size','minimize','restore','state')][string]$Action='state',[int]$ClientWidth=1280,[int]$ClientHeight=900)
$ErrorActionPreference='Stop'
Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
public static class NativeReaderProbe {
 [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left,Top,Right,Bottom; }
 [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr w,out Rect r);
 [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr w,out Rect r);
 [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr w);
 [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr w,int n);
 [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr w,IntPtr after,int x,int y,int cx,int cy,uint flags);
 [DllImport("user32.dll")] public static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);
 public delegate bool WindowCallback(IntPtr w,IntPtr data);
 [DllImport("user32.dll")] public static extern bool EnumWindows(WindowCallback callback,IntPtr data);
 [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr w,out uint pid);
 [DllImport("user32.dll",CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr w,System.Text.StringBuilder name,int count);
 public static IntPtr ReaderWindow(uint pid) { IntPtr result=IntPtr.Zero; EnumWindows((w,data)=>{uint found; GetWindowThreadProcessId(w,out found); if(found!=pid)return true; var name=new System.Text.StringBuilder(256); GetClassName(w,name,256); if(name.ToString()=="QuranMushafReader.Rust"){result=w;return false;}return true;},IntPtr.Zero); return result; }
}
"@
[NativeReaderProbe]::SetThreadDpiAwarenessContext([IntPtr](-4))|Out-Null
$hostProcess=Get-Process -Id $HostProcessId
$window=[NativeReaderProbe]::ReaderWindow([uint32]$HostProcessId)
if($window -eq 0){throw 'The isolated QA host has no window'}
if($Action -eq 'restore'){[NativeReaderProbe]::ShowWindow($window,9)|Out-Null}
if($Action -eq 'minimize'){[NativeReaderProbe]::ShowWindow($window,6)|Out-Null}
if($Action -eq 'size'){
 [NativeReaderProbe]::ShowWindow($window,9)|Out-Null
 $client=New-Object NativeReaderProbe+Rect; $outer=New-Object NativeReaderProbe+Rect
 [NativeReaderProbe]::GetClientRect($window,[ref]$client)|Out-Null
 [NativeReaderProbe]::GetWindowRect($window,[ref]$outer)|Out-Null
 $width=$ClientWidth+($outer.Right-$outer.Left)-($client.Right-$client.Left)
 $height=$ClientHeight+($outer.Bottom-$outer.Top)-($client.Bottom-$client.Top)
 [NativeReaderProbe]::SetWindowPos($window,[IntPtr]::Zero,20,20,$width,$height,0x0014)|Out-Null
}
$client=New-Object NativeReaderProbe+Rect
[NativeReaderProbe]::GetClientRect($window,[ref]$client)|Out-Null
[pscustomobject]@{pid=$HostProcessId;minimized=[NativeReaderProbe]::IsIconic($window);clientWidth=$client.Right-$client.Left;clientHeight=$client.Bottom-$client.Top}|ConvertTo-Json -Compress
