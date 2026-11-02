// Flat peanut-butter smear: organic knife edges, a few spread ridges, and dark outlined lettering.
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
  <path d="M13 26C23 12 46 6 71 9C105 5 136 9 162 7C187 5 211 10 216 19C219 24 211 27 204 29L215 32C223 36 218 41 209 44L217 49C224 53 217 59 209 62C219 66 216 72 207 76L213 81C218 86 207 92 199 94C208 101 195 109 183 111C174 122 146 124 125 122C98 128 70 125 48 127C29 128 12 119 11 108C3 100 5 88 7 78C1 67 4 55 7 47C4 38 8 32 13 26Z" fill="#D5A263"/>
  <path d="M15 29C28 17 52 14 80 16C111 12 141 17 166 14C187 12 201 14 207 18C195 16 181 20 165 20C135 22 110 18 81 22C53 19 30 22 15 29Z" fill="#E9BC7B"/>
  <path d="M15 94C13 107 29 118 49 118C74 116 100 121 125 116C149 119 171 115 183 110C168 120 143 124 125 122C98 128 70 125 48 127C29 128 12 119 11 108C8 102 9 98 15 94Z" fill="#BC803E"/>
  <path d="M197 33c-9 5-12 11-11 19m20-2c-11 6-17 16-16 26M25 102c17 9 36 7 51 8m84-13c17-2 25-6 31-13" fill="none" stroke="#BC803E" stroke-width="2.5" stroke-linecap="round"/>
  <path d="M16 42c-4 9-5 18-2 27m5 8c-2 8-1 13 2 17" fill="none" stroke="#E9BC7B" stroke-width="3" stroke-linecap="round"/>
  <path transform="translate(22 49) scale(1 -1)" d="\(peanut)" fill="#382719"/>
  <path transform="translate(42 87) scale(1 -1)" d="\(butter)" fill="#382719"/>
  <path transform="translate(57 108) scale(1 -1)" d="\(passwords)" fill="#382719"/>
</svg>
"""
try svg.write(toFile: "resources/brand/wordmark.svg", atomically: true, encoding: .utf8)
