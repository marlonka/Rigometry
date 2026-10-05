#Requires -Version 7.0
# Rasterize the authored SVG's rect/polyline subset. No downloaded artwork.
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
$assetPath = Join-Path $PSScriptRoot '../assets'
[xml]$svg = Get-Content -LiteralPath (Join-Path $assetPath 'rigometry.svg') -Raw

function New-IconPng([int]$Size) {
    $factor = 4
    $canvas = [Drawing.Bitmap]::new($Size * $factor, $Size * $factor)
    $graphics = [Drawing.Graphics]::FromImage($canvas)
    try {
        $graphics.Clear([Drawing.Color]::Transparent)
        $graphics.SmoothingMode = [Drawing.Drawing2D.SmoothingMode]::AntiAlias
        $graphics.ScaleTransform($Size * $factor / 64.0, $Size * $factor / 64.0)
        foreach ($node in $svg.DocumentElement.ChildNodes) {
            $fill = $node.GetAttribute('fill')
            $stroke = $node.GetAttribute('stroke')
            $shape = [Drawing.Drawing2D.GraphicsPath]::new()
            try {
                switch ($node.LocalName) {
                    'rect' {
                        $diameter = [float]$node.rx * 2
                        $x = [float]$node.x
                        $y = [float]$node.y
                        $width = [float]$node.width
                        $height = [float]$node.height
                        $shape.AddArc($x, $y, $diameter, $diameter, 180, 90)
                        $shape.AddArc($x + $width - $diameter, $y, $diameter, $diameter, 270, 90)
                        $shape.AddArc($x + $width - $diameter, $y + $height - $diameter, $diameter, $diameter, 0, 90)
                        $shape.AddArc($x, $y + $height - $diameter, $diameter, $diameter, 90, 90)
                        $shape.CloseFigure()
                    }
                    'polyline' {
                        [Drawing.PointF[]]$points = foreach ($pair in $node.points.Split(' ', [StringSplitOptions]::RemoveEmptyEntries)) {
                            $coordinates = $pair.Split(',')
                            [Drawing.PointF]::new([float]$coordinates[0], [float]$coordinates[1])
                        }
                        $shape.AddLines($points)
                    }
                    default { throw "Unsupported SVG element: $($node.LocalName)" }
                }
                if ($fill -and $fill -ne 'none') {
                    $brush = [Drawing.SolidBrush]::new([Drawing.ColorTranslator]::FromHtml($fill))
                    try { $graphics.FillPath($brush, $shape) } finally { $brush.Dispose() }
                }
                if ($stroke -and $stroke -ne 'none') {
                    $pen = [Drawing.Pen]::new([Drawing.ColorTranslator]::FromHtml($stroke), [float]::Parse($node.GetAttribute('stroke-width'), [Globalization.CultureInfo]::InvariantCulture))
                    try {
                        $pen.StartCap = [Drawing.Drawing2D.LineCap]::Round
                        $pen.EndCap = [Drawing.Drawing2D.LineCap]::Round
                        $pen.LineJoin = [Drawing.Drawing2D.LineJoin]::Round
                        $graphics.DrawPath($pen, $shape)
                    } finally { $pen.Dispose() }
                }
            } finally {
                $shape.Dispose()
            }
        }
        $output = [Drawing.Bitmap]::new($Size, $Size)
        $scaled = [Drawing.Graphics]::FromImage($output)
        $stream = [IO.MemoryStream]::new()
        try {
            $scaled.InterpolationMode = [Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
            $scaled.PixelOffsetMode = [Drawing.Drawing2D.PixelOffsetMode]::HighQuality
            $scaled.DrawImage($canvas, 0, 0, $Size, $Size)
            $output.Save($stream, [Drawing.Imaging.ImageFormat]::Png)
            return ,$stream.ToArray()
        } finally {
            $stream.Dispose()
            $scaled.Dispose()
            $output.Dispose()
        }
    } finally {
        $graphics.Dispose()
        $canvas.Dispose()
    }
}

$sizes = @(16, 24, 32, 48, 64, 128, 256)
$images = @($sizes | ForEach-Object { New-IconPng $_ })
[IO.File]::WriteAllBytes((Join-Path $assetPath 'rigometry.png'), (New-IconPng 512))
$ico = [IO.MemoryStream]::new()
$writer = [IO.BinaryWriter]::new($ico)
try {
    $writer.Write([uint16]0)
    $writer.Write([uint16]1)
    $writer.Write([uint16]$sizes.Count)
    $offset = 6 + 16 * $sizes.Count
    for ($i = 0; $i -lt $sizes.Count; $i++) {
        $dimension = if ($sizes[$i] -eq 256) { 0 } else { $sizes[$i] }
        $writer.Write([byte]$dimension)
        $writer.Write([byte]$dimension)
        $writer.Write([uint16]0)
        $writer.Write([uint16]1)
        $writer.Write([uint16]32)
        $writer.Write([uint32]$images[$i].Length)
        $writer.Write([uint32]$offset)
        $offset += $images[$i].Length
    }
    foreach ($bytes in $images) { $writer.Write([byte[]]$bytes) }
    [IO.File]::WriteAllBytes((Join-Path $assetPath 'rigometry.ico'), $ico.ToArray())
} finally {
    $writer.Dispose()
    $ico.Dispose()
}
