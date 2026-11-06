# V-Notch Legion helper. Runs elevated (scheduled task "V-Notch Legion", started at logon) because
# Lenovo's GameZone WMI interface only answers administrators. Serves \\.\pipe\vnotch-legion to the
# one user given by -User: one request line, one JSON answer line, then the pipe is closed.
# Only the commands below exist, so a caller can do no more than Fn+Q or Lenovo Vantage can.
param([Parameter(Mandatory = $true)][string]$User)

$ErrorActionPreference = 'Stop'
$owner = New-Object System.Security.Principal.SecurityIdentifier($User)
$security = New-Object System.IO.Pipes.PipeSecurity
$security.AddAccessRule((New-Object System.IO.Pipes.PipeAccessRule($owner, 'ReadWrite', 'Allow')))
$security.AddAccessRule((New-Object System.IO.Pipes.PipeAccessRule(
    (New-Object System.Security.Principal.SecurityIdentifier('S-1-5-32-544')), 'FullControl', 'Allow')))

$script:gz = $null
function Invoke-GameZone([string]$method, $data) {
    for ($attempt = 0; $attempt -lt 2; $attempt++) {
        try {
            if ($null -eq $script:gz) {
                $script:gz = Get-WmiObject -Namespace root\WMI -Class LENOVO_GAMEZONE_DATA | Select-Object -First 1
            }
            if ($null -eq $data) { return [int]$script:gz.$method().Data }
            [void]$script:gz.$method([uint32]$data)
            return 0
        } catch { $script:gz = $null }
    }
    return -1
}

function Get-State {
    [ordered]@{
        version  = 2 # LegionService.HelperVersion: an older copy makes the notch offer the setup again
        mode     = Invoke-GameZone 'GetSmartFanMode' $null
        winKey   = Invoke-GameZone 'GetWinKeyStatus' $null
        touchpad = Invoke-GameZone 'GetTPStatus' $null
        cpuTemp  = Invoke-GameZone 'GetCPUTemp' $null
        gpuTemp  = Invoke-GameZone 'GetGPUTemp' $null
    }
}

$setters = @{ mode = 'SetSmartFanMode'; winkey = 'SetWinKeyStatus'; touchpad = 'SetTPStatus' }
# 1 quiet, 2 balanced, 3 performance, 224 extreme, 255 custom
$allowed = @{ mode = @(1, 2, 3, 224, 255); winkey = @(0, 1); touchpad = @(0, 1) }

while ($true) {
    # Throws when another helper already serves the pipe: then this one simply ends.
    $pipe = New-Object System.IO.Pipes.NamedPipeServerStream('vnotch-legion', 'InOut', 1, 'Byte', 'None', 1024, 4096, $security)
    try {
        $pipe.WaitForConnection()
        $reader = New-Object System.IO.StreamReader($pipe)
        $writer = New-Object System.IO.StreamWriter($pipe)
        $request = "$($reader.ReadLine())".Trim().ToLowerInvariant() -split ' '
        $answer = $null
        if ($request.Count -eq 2 -and $setters.ContainsKey($request[0])) {
            $value = 0
            if ([int]::TryParse($request[1], [ref]$value) -and $allowed[$request[0]] -contains $value) {
                if ((Invoke-GameZone $setters[$request[0]] $value) -lt 0) { $answer = @{ error = 'failed' } }
            } else { $answer = @{ error = 'bad value' } }
        } elseif ($request[0] -ne 'state') { $answer = @{ error = 'unknown command' } }
        if ($null -eq $answer) { $answer = Get-State }
        $writer.WriteLine(($answer | ConvertTo-Json -Compress))
        $writer.Flush()
        $pipe.WaitForPipeDrain()
    } catch {
    } finally {
        $pipe.Dispose()
    }
}
