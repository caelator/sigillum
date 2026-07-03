#!/usr/bin/env swift
import AppKit
import CoreGraphics

let canvasSide = 1024
let canvasSize = CGSize(width: canvasSide, height: canvasSide)
let canvasRect = CGRect(origin: .zero, size: canvasSize)

func absoluteURL(for path: String, relativeTo baseURL: URL) -> URL {
    if path.hasPrefix("/") {
        return URL(fileURLWithPath: path).standardizedFileURL
    }
    return URL(fileURLWithPath: path, relativeTo: baseURL).standardizedFileURL
}

let currentDirectoryURL = URL(fileURLWithPath: FileManager.default.currentDirectoryPath, isDirectory: true)
let scriptURL = absoluteURL(for: CommandLine.arguments[0], relativeTo: currentDirectoryURL)
let repoRootURL = scriptURL
    .deletingLastPathComponent()
    .deletingLastPathComponent()
    .deletingLastPathComponent()

let outputURL: URL
if CommandLine.arguments.count > 1 {
    outputURL = absoluteURL(for: CommandLine.arguments[1], relativeTo: currentDirectoryURL)
} else {
    outputURL = repoRootURL
        .appendingPathComponent("crates")
        .appendingPathComponent("sigillum-desktop")
        .appendingPathComponent("icons")
        .appendingPathComponent("master.png")
}

guard let bitmap = NSBitmapImageRep(
    bitmapDataPlanes: nil,
    pixelsWide: canvasSide,
    pixelsHigh: canvasSide,
    bitsPerSample: 8,
    samplesPerPixel: 4,
    hasAlpha: true,
    isPlanar: false,
    colorSpaceName: .deviceRGB,
    bytesPerRow: 0,
    bitsPerPixel: 0
) else {
    fatalError("Unable to allocate bitmap")
}

bitmap.size = canvasSize

guard let context = NSGraphicsContext(bitmapImageRep: bitmap) else {
    fatalError("Unable to create graphics context")
}

NSGraphicsContext.saveGraphicsState()
NSGraphicsContext.current = context

let cg = context.cgContext
cg.setAllowsAntialiasing(true)
cg.setShouldAntialias(true)
cg.clear(canvasRect)

let backgroundColor = CGColor(srgbRed: 0x0d / 255.0, green: 0x11 / 255.0, blue: 0x17 / 255.0, alpha: 1.0)
let foregroundColor = CGColor(srgbRed: 0xe6 / 255.0, green: 0xed / 255.0, blue: 0xf3 / 255.0, alpha: 1.0)
let foregroundColorMuted = CGColor(srgbRed: 0xe6 / 255.0, green: 0xed / 255.0, blue: 0xf3 / 255.0, alpha: 0.55)

let backgroundRect = CGRect(x: 64, y: 64, width: 896, height: 896)
cg.addPath(CGPath(roundedRect: backgroundRect, cornerWidth: 200, cornerHeight: 200, transform: nil))
cg.setFillColor(backgroundColor)
cg.fillPath()

let sealCenter = CGPoint(x: 512, y: 512)
func ringRect(radius: CGFloat) -> CGRect {
    CGRect(x: sealCenter.x - radius, y: sealCenter.y - radius, width: radius * 2, height: radius * 2)
}

cg.setStrokeColor(foregroundColor)
cg.setLineWidth(14)
cg.strokeEllipse(in: ringRect(radius: 330))

cg.setStrokeColor(foregroundColorMuted)
cg.setLineWidth(4)
cg.strokeEllipse(in: ringRect(radius: 300))

let font = NSFont.systemFont(ofSize: 430, weight: .bold)
let monogramColor = NSColor(srgbRed: 0xe6 / 255.0, green: 0xed / 255.0, blue: 0xf3 / 255.0, alpha: 1.0)
let monogram = NSAttributedString(string: "S", attributes: [
    .font: font,
    .foregroundColor: monogramColor
])

let monogramSize = monogram.size()
let opticalYOffset: CGFloat = -18
let monogramOrigin = CGPoint(
    x: (canvasSize.width - monogramSize.width) / 2,
    y: (canvasSize.height - monogramSize.height) / 2 + opticalYOffset
)
monogram.draw(at: monogramOrigin)

NSGraphicsContext.restoreGraphicsState()

try FileManager.default.createDirectory(at: outputURL.deletingLastPathComponent(), withIntermediateDirectories: true)

guard let pngData = bitmap.representation(using: .png, properties: [:]) else {
    fatalError("Unable to encode PNG")
}

try pngData.write(to: outputURL, options: .atomic)
print("Wrote \(outputURL.path)")
