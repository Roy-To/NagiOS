#!/usr/bin/env swift
// Generate the native bitmap subset used by nagi-init's software Painter.
// Source: Noto Sans JP Regular from third_party/fonts.lock (OFL-1.1).
// The generated table is limited to the Japanese 0.1 UI catalogs plus the
// dynamic Calendar date/time examples covered by font.rs host regressions.

import CoreGraphics
import CoreText
import CryptoKit
import Foundation

let fontURL = URL(fileURLWithPath: CommandLine.arguments[1])
let outputURL = URL(fileURLWithPath: CommandLine.arguments[2])
let repositoryURL = URL(fileURLWithPath: CommandLine.arguments[3], isDirectory: true)
let expectedSize = 4_533_028
let expectedSHA256 = "dff723ba59d57d136764a04b9b2d03205544f7cd785a711442d6d2d085ac5073"

let fontData = try Data(contentsOf: fontURL)
let actualSHA256 = SHA256.hash(data: fontData).map { String(format: "%02x", $0) }.joined()
guard fontData.count == expectedSize, actualSHA256 == expectedSHA256 else {
    fatalError("Noto Sans JP input does not match third_party/fonts.lock")
}
guard CTFontManagerRegisterFontsForURL(fontURL as CFURL, .process, nil) else {
    fatalError("could not register pinned Noto Sans JP font")
}
let latinFont = CTFontCreateWithName("Noto Sans JP" as CFString, 7, nil)
let japaneseFont = CTFontCreateWithName("Noto Sans JP" as CFString, 9, nil)
guard String(CTFontCopyPostScriptName(latinFont)) == "NotoSansJP-Regular",
      String(CTFontCopyPostScriptName(japaneseFont)) == "NotoSansJP-Regular" else {
    fatalError("CoreText did not load the pinned Noto Sans JP Regular face")
}

let relativeCatalogs = [
    "user/nagi-localization/locales/ja-JP.lang",
    "user/nagi-notes/locales/ja-JP.properties",
    "apps/nagi-files/locales/ja-JP.properties",
    "crates/nagi-i18n/locales/ja-JP.json",
]
var textValues: [String] = []
for relativePath in relativeCatalogs {
    let url = repositoryURL.appendingPathComponent(relativePath)
    let contents = try String(contentsOf: url, encoding: .utf8)
    if relativePath.hasSuffix(".json") {
        let json = try JSONSerialization.jsonObject(with: Data(contents.utf8))
        guard let root = json as? [String: Any],
              let messages = root["messages"] as? [String: Any] else {
            fatalError("Japanese catalog has no messages object: \(relativePath)")
        }
        textValues += messages.values.compactMap { $0 as? String }
    } else {
        textValues += contents.split(whereSeparator: \.isNewline).compactMap { line in
            let trimmed = line.trimmingCharacters(in: .whitespaces)
            guard !trimmed.isEmpty, !trimmed.hasPrefix("#"),
                  let separator = trimmed.firstIndex(of: "=") else { return nil }
            return String(trimmed[trimmed.index(after: separator)...]).trimmingCharacters(in: .whitespaces)
        }
    }
}

// Calendar formats dates at runtime, so include its Japanese date form and
// the exact ASCII date/time/indicator glyphs exercised by C7.
textValues += [
    "日本語", "設定", "パスワード変更", "カレンダー", "ログアウト",
    "2026年3月5日", "2026年11月23日", "2026年11月24日",
    "09:00-10:00 event a", "23:00-01:00+ event c",
    "-23:00-01:00 event b edited", "< Previous", "Next >",
    "表示できません",
]

let japaneseScalars = Set(textValues.flatMap { $0.unicodeScalars.map(\.value) }
    .filter { $0 > 0x7f })
let printableASCII = Array(UInt32(0x20)...UInt32(0x7e))

func bitmap(for scalar: UInt32, font: CTFont, width: Int, height: Int) -> [UInt16] {
    guard let unicode = UnicodeScalar(scalar), scalar <= 0xffff else {
        fatalError(String(format: "unsupported non-BMP UI scalar U+%04X", scalar))
    }
    let utf16 = Array(String(unicode).utf16)
    let characterCount = utf16.count
    var glyphs = [CGGlyph](repeating: 0, count: characterCount)
    let found = utf16.withUnsafeBufferPointer { characters in
        glyphs.withUnsafeMutableBufferPointer { glyphBuffer in
            CTFontGetGlyphsForCharacters(font, characters.baseAddress!, glyphBuffer.baseAddress!, characterCount)
        }
    }
    guard found, glyphs.first != 0 else {
        fatalError(String(format: "pinned Noto font has no glyph U+%04X", scalar))
    }

    var pixels = [UInt8](repeating: 255, count: width * height)
    pixels.withUnsafeMutableBytes { raw in
        guard let context = CGContext(
            data: raw.baseAddress,
            width: width,
            height: height,
            bitsPerComponent: 8,
            bytesPerRow: width,
            space: CGColorSpaceCreateDeviceGray(),
            bitmapInfo: CGImageAlphaInfo.none.rawValue
        ) else { fatalError("could not allocate glyph raster context") }
        context.setFillColor(gray: 1, alpha: 1)
        context.fill(CGRect(x: 0, y: 0, width: width, height: height))
        context.setFillColor(gray: 0, alpha: 1)
        context.translateBy(x: 0, y: CGFloat(height))
        context.scaleBy(x: 1, y: -1)
        let attributes: [NSAttributedString.Key: Any] = [
            NSAttributedString.Key(kCTFontAttributeName as String): font,
        ]
        let line = CTLineCreateWithAttributedString(
            NSAttributedString(string: String(unicode), attributes: attributes)
        )
        CTLineDraw(line, context)
    }

    return (0..<height).map { y in
        (0..<width).reduce(UInt16(0)) { row, x in
            pixels[y * width + x] < 160 ? row | (UInt16(1) << (width - 1 - x)) : row
        }
    }
}

var generated = """
// Generated by tools/fonts/generate_native_bitmap_font.swift.
// Source: Noto Sans JP Regular, revision f8d157532fbfaeda587e826d4cd5b21a49186f7c.
// Source SHA-256: \(expectedSHA256); license: SIL Open Font License 1.1.
// Do not edit by hand; regenerate from the pinned font and locale catalogs.
pub(super) const ASCII_FALLBACK_GLYPHS: &[(u8, [u8; 7])] = &[
"""
generated += "\n"
for scalar in printableASCII {
    let bitmap = bitmap(for: scalar, font: latinFont, width: 6, height: 7)
    let array = bitmap.map { String(format: "0x%02x", $0) }.joined(separator: ", ")
    generated += String(format: "    (0x%02x, [%@]),\n", scalar, array)
}
generated += "];\n\npub(super) const JAPANESE_GLYPHS: &[(u32, [u16; 9])] = &[\n"
for scalar in japaneseScalars.sorted() {
    let bitmap = bitmap(for: scalar, font: japaneseFont, width: 9, height: 9)
    let character = String(UnicodeScalar(scalar)!)
    let rowValues = bitmap.map { String(format: "0x%03x", $0) }.joined(separator: ", ")
    generated += String(
        format: "    (\n        0x%04x,\n        [\n            %@,\n        ],\n    ), // %@\n",
        scalar,
        rowValues,
        character
    )
}
generated += "];\n"
try generated.write(to: outputURL, atomically: true, encoding: .utf8)
print("generated \(printableASCII.count) ASCII fallbacks and \(japaneseScalars.count) Japanese UI glyphs")
