// Flat illustrated wordmark: outlined lettering, a spread-shaped label, and peanut detail.
// Run from the project root:
// swift -module-cache-path /private/tmp/pb-passwords-swift-cache assets/generate-wordmark.swift
import Foundation
import CoreText
import CoreGraphics

func outline(_ text: String, font name: String, size: CGFloat) -> String {
    let font = CTFontCreateWithName(name as CFString, size, nil)
    let characters = Array(text.utf16)
    var glyphs = Array(repeating: CGGlyph(0), count: characters.count)
    var advances = Array(repeating: CGSize.zero, count: characters.count)
    CTFontGetGlyphsForCharacters(font, characters, &glyphs, characters.count)
    CTFontGetAdvancesForGlyphs(font, .horizontal, glyphs, &advances, glyphs.count)
    let combined = CGMutablePath()
    var x: CGFloat = 0
    for (index, glyph) in glyphs.enumerated() {
        if let path = CTFontCreatePathForGlyph(font, glyph, nil) {
            combined.addPath(path, transform: CGAffineTransform(translationX: x, y: 0))
        }
        x += advances[index].width
    }
    func point(_ point: CGPoint) -> String {
        String(format: "%.3f %.3f", Double(point.x), Double(point.y))
    }
    var data = ""
    combined.applyWithBlock { element in
        let value = element.pointee
        switch value.type {
        case .moveToPoint: data += "M" + point(value.points[0])
        case .addLineToPoint: data += "L" + point(value.points[0])
        case .addQuadCurveToPoint: data += "Q" + point(value.points[0]) + " " + point(value.points[1])
        case .addCurveToPoint: data += "C" + point(value.points[0]) + " " + point(value.points[1]) + " " + point(value.points[2])
        case .closeSubpath: data += "Z"
        @unknown default: break
        }
    }
    return data
}

let font = "ArialRoundedMTBold"
let peanut = outline("peanut", font: font, size: 44)
let butter = outline("butter", font: font, size: 44)
let passwords = outline("PASSWORDS", font: font, size: 13)
let svg = """
<svg xmlns="http://www.w3.org/2000/svg" width="224" height="134" viewBox="0 0 224 134">
  <path d="M21 6Q12 4 8 15L4 77Q3 100 26 100L185 100Q208 99 211 80L215 27Q216 8 193 7Z" fill="#925B2B"/>
  <path d="M19 2Q10 1 6 12L2 72Q1 94 24 94L182 94Q204 93 207 75L211 22Q212 3 189 2Z" fill="#BC803E"/>
  <path d="M18 11H187" stroke="#D5A263" stroke-width="4" stroke-linecap="round"/>
  <path transform="translate(18 47) scale(1 -1)" d="\(peanut)" fill="#59412A"/>
  <path transform="translate(16 45) scale(1 -1)" d="\(peanut)" fill="#FAE3BB"/>
  <path transform="translate(38 87) scale(1 -1)" d="\(butter)" fill="#59412A"/>
  <path transform="translate(36 85) scale(1 -1)" d="\(butter)" fill="#F4DBAD"/>
  <path transform="translate(30 123) scale(1 -1)" d="\(passwords)" fill="#E9BC7B"/>
  <g transform="translate(168 102) rotate(24 12 12)">
    <path d="M10 1C3 1 0 6 3 12 5 17 1 22 7 26 13 30 21 26 20 19 19 14 15 13 17 8 18 3 14 1 10 1Z" fill="#E9BC7B"/>
    <path d="m6 6 9 15M5 13l9-7M7 21l10-7" fill="none" stroke="#BC803E" stroke-width="2" stroke-linecap="round"/>
  </g>
</svg>
"""
try svg.write(toFile: "resources/brand/wordmark.svg", atomically: true, encoding: .utf8)
