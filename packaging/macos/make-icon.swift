// Draws the app icon (navy rounded square, white envelope) and writes a 1024x1024 PNG.
// Original artwork, drawn in code. Usage: swift make-icon.swift out.png
import AppKit
let out = CommandLine.arguments.count > 1 ? CommandLine.arguments[1] : "icon.png"
let size = 1024.0
let image = NSImage(size: NSSize(width: size, height: size))
image.lockFocus()
let ctx = NSGraphicsContext.current!.cgContext
ctx.clear(CGRect(x: 0, y: 0, width: size, height: size))
// background: rounded square with a soft vertical gradient
let inset = size * 0.08
let bg = NSBezierPath(roundedRect: NSRect(x: inset, y: inset, width: size - 2 * inset, height: size - 2 * inset), xRadius: size * 0.2, yRadius: size * 0.2)
let gradient = NSGradient(colors: [NSColor(red: 0.21, green: 0.47, blue: 0.93, alpha: 1), NSColor(red: 0.11, green: 0.23, blue: 0.39, alpha: 1)])!
gradient.draw(in: bg, angle: -90)
// envelope
let w = size * 0.56, h = size * 0.40
let x = (size - w) / 2, y = (size - h) / 2
let env = NSBezierPath(roundedRect: NSRect(x: x, y: y, width: w, height: h), xRadius: size * 0.04, yRadius: size * 0.04)
NSColor.white.setFill(); env.fill()
// flap (V) in the background colour
let flap = NSBezierPath()
flap.move(to: NSPoint(x: x + size * 0.02, y: y + h - size * 0.02))
flap.line(to: NSPoint(x: x + w / 2, y: y + h * 0.42))
flap.line(to: NSPoint(x: x + w - size * 0.02, y: y + h - size * 0.02))
flap.lineWidth = size * 0.045
flap.lineCapStyle = .round; flap.lineJoinStyle = .round
NSColor(red: 0.13, green: 0.28, blue: 0.47, alpha: 1).setStroke(); flap.stroke()
// unread dot
let dot = NSBezierPath(ovalIn: NSRect(x: x + w - size * 0.07, y: y + h - size * 0.07, width: size * 0.14, height: size * 0.14))
NSColor(red: 0.96, green: 0.62, blue: 0.04, alpha: 1).setFill(); dot.fill()
image.unlockFocus()
let rep = NSBitmapImageRep(data: image.tiffRepresentation!)!
let png = rep.representation(using: .png, properties: [:])!
try! png.write(to: URL(fileURLWithPath: out))
print(out)
