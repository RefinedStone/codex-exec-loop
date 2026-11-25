# Prepare only a new disposable-user fixture; the production helper retains its
# own executable/ancestor/ACL/native-image checks when the operator launches it.
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string] $ExpectedSid,
    [Parameter(Mandatory = $true)][string] $GatewayExecutable,
    [Parameter(Mandatory = $true)][string] $RunId
)
$ErrorActionPreference = 'Stop'
# The Node caller decodes the JSON path as UTF-8, independent of the OEM code page.
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
if ($RunId -notmatch '^[a-f0-9]{32}$') { throw 'Invalid fixture run id.' }
$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
if ($identity.User.Value -cne $ExpectedSid) { throw 'Disposable OS user SID mismatch.' }
if (-not [Environment]::UserInteractive) { throw 'An interactive disposable OS user is required.' }
$profile = [Environment]::GetFolderPath([Environment+SpecialFolder]::UserProfile)
if (-not [IO.Path]::IsPathRooted($profile)) { throw 'The OS profile could not be resolved.' }
if (-not [IO.Path]::IsPathRooted($GatewayExecutable)) { throw 'An absolute helper source is required.' }
$root = Join-Path $profile ('akra-vault-live-' + $RunId)
if ([IO.Directory]::Exists($root) -or [IO.File]::Exists($root)) { throw 'Never reuse or repair an existing fixture root.' }
$security = [Security.AccessControl.DirectorySecurity]::new()
$security.SetOwner($identity.User)
$security.SetAccessRuleProtection($true, $false)
$rule = [Security.AccessControl.FileSystemAccessRule]::new(
    $identity.User,
    [Security.AccessControl.FileSystemRights]::FullControl,
    ([Security.AccessControl.InheritanceFlags]::ContainerInherit -bor [Security.AccessControl.InheritanceFlags]::ObjectInherit),
    [Security.AccessControl.PropagationFlags]::None,
    [Security.AccessControl.AccessControlType]::Allow
)
$security.AddAccessRule($rule)
$source = $null
$destination = $null
try {
    # Holding this read handle without write/delete sharing fixes the copied bytes.
    $source = [IO.File]::Open($GatewayExecutable, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
    if ($source.ReadByte() -ne 77 -or $source.ReadByte() -ne 90) { throw 'The helper source is not a native Windows image.' }
    $source.Position = 0
    [void][IO.Directory]::CreateDirectory($root, $security)
    foreach ($name in @('helper', 'akra-home', 'electron-user-data', 'captures')) {
        [void][IO.Directory]::CreateDirectory((Join-Path $root $name), $security)
    }
    $target = Join-Path (Join-Path $root 'helper') 'akra-gateway.exe'
    $fileSecurity = [Security.AccessControl.FileSecurity]::new()
    $fileSecurity.SetOwner($identity.User)
    $fileSecurity.SetAccessRuleProtection($true, $false)
    $fileSecurity.AddAccessRule([Security.AccessControl.FileSystemAccessRule]::new($identity.User, [Security.AccessControl.FileSystemRights]::FullControl, [Security.AccessControl.AccessControlType]::Allow))
    $destination = [IO.FileStream]::new($target, [IO.FileMode]::CreateNew, [Security.AccessControl.FileSystemRights]::FullControl, [IO.FileShare]::None, 4096, [IO.FileOptions]::WriteThrough, $fileSecurity)
    $source.CopyTo($destination)
    $destination.Flush($true)
    $source.Position = 0
    $destination.Position = 0
    $sha = [Security.Cryptography.SHA256]::Create()
    try {
        $before = [BitConverter]::ToString($sha.ComputeHash($source))
        $after = [BitConverter]::ToString($sha.ComputeHash($destination))
        if ($before -cne $after) { throw 'The helper copy differs from its locked source.' }
    } finally { $sha.Dispose() }
} finally {
    if ($null -ne $destination) { $destination.Dispose() }
    if ($null -ne $source) { $source.Dispose() }
}
# No credential call, launch, existing-parent ACL repair, or automatic cleanup.
@{ root = $root } | ConvertTo-Json -Compress
