# Builds mdedit.ico from the eight glyph PNGs.
#
# The icon pack's own README says to run ImageMagick. ImageMagick is not a dependency of this
# repo and is not on the CI runners, so the .ico is built by this script instead and the result
# is committed. Everything here is System.Drawing, which is present on every Windows.
#
# Sizes below 256 are written as BMP DIBs and 256 as a PNG. Windows has accepted PNG entries at
# any size since Vista, but the shell's older code paths are only reliably exercised for BMP
# below 256 — and an icon that fails to draw in one context is expensive to find later.

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$sizes = @(16, 20, 24, 32, 48, 64, 128, 256)
$out = Join-Path $here 'mdedit.ico'

# One entry's pixels as a bottom-up 32bpp DIB: BITMAPINFOHEADER with a doubled height, the BGRA
# rows in reverse, then the AND mask. The mask is all-zero because the alpha channel is what
# actually cuts the icon out; it is present because the format requires it.
function ConvertTo-Dib([System.Drawing.Bitmap]$bitmap) {
    $width = $bitmap.Width
    $height = $bitmap.Height
    $stream = New-Object System.IO.MemoryStream
    $writer = New-Object System.IO.BinaryWriter $stream

    $writer.Write([uint32]40)
    $writer.Write([int32]$width)
    $writer.Write([int32]($height * 2))
    $writer.Write([uint16]1)
    $writer.Write([uint16]32)
    $writer.Write([uint32]0)
    $writer.Write([uint32]($width * $height * 4))
    0..3 | ForEach-Object { $writer.Write([uint32]0) }

    for ($y = $height - 1; $y -ge 0; $y--) {
        for ($x = 0; $x -lt $width; $x++) {
            $pixel = $bitmap.GetPixel($x, $y)
            $writer.Write([byte]$pixel.B)
            $writer.Write([byte]$pixel.G)
            $writer.Write([byte]$pixel.R)
            $writer.Write([byte]$pixel.A)
        }
    }

    # Mask rows are one bit per pixel, padded to a four-byte boundary.
    $maskRow = [math]::Floor(($width + 31) / 32) * 4
    $writer.Write((New-Object byte[] ($maskRow * $height)))

    $writer.Flush()
    $bytes = $stream.ToArray()
    $writer.Dispose(); $stream.Dispose()
    # Comma-wrapped: PowerShell unrolls a returned array into the pipeline, which turns a byte[]
    # into an Object[] and stops BinaryWriter.Write from matching its byte[] overload.
    , $bytes
}

$images = foreach ($size in $sizes) {
    $path = Join-Path $here "mdedit-$size.png"
    if (-not (Test-Path $path)) { throw "missing $path" }
    $bitmap = New-Object System.Drawing.Bitmap $path
    if ($bitmap.Width -ne $size -or $bitmap.Height -ne $size) {
        throw "$path is $($bitmap.Width)x$($bitmap.Height), expected ${size}x${size}"
    }
    [byte[]]$data = if ($size -eq 256) { [System.IO.File]::ReadAllBytes($path) } else { ConvertTo-Dib $bitmap }
    $bitmap.Dispose()
    [pscustomobject]@{ Size = $size; Data = $data }
}

$stream = [System.IO.File]::Create($out)
$writer = New-Object System.IO.BinaryWriter $stream

# ICONDIR, then one ICONDIRENTRY per image, then the images themselves.
$writer.Write([uint16]0)
$writer.Write([uint16]1)
$writer.Write([uint16]$images.Count)

$offset = 6 + 16 * $images.Count
foreach ($image in $images) {
    # 256 is written as 0: the field is one byte and the format says so.
    $writer.Write([byte]($image.Size % 256))
    $writer.Write([byte]($image.Size % 256))
    $writer.Write([byte]0)
    $writer.Write([byte]0)
    $writer.Write([uint16]1)
    $writer.Write([uint16]32)
    $writer.Write([uint32]$image.Data.Length)
    $writer.Write([uint32]$offset)
    $offset += $image.Data.Length
}
foreach ($image in $images) { $writer.Write($image.Data) }

$writer.Flush(); $writer.Dispose(); $stream.Dispose()
"wrote $out ($((Get-Item $out).Length) bytes, $($images.Count) images)"
