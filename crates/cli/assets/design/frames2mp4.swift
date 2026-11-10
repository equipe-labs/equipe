// `equipe design flows record`, on a Mac without ffmpeg: frames to an H.264
// .mp4 through AVFoundation. The list has a line per frame, `<ms>\t<path>`,
// in order, and last `<ms>\t` for where the video ends.
//
//     swift frames2mp4.swift <list> <out.mp4>
import AVFoundation
import CoreGraphics
import CoreVideo
import Foundation
import ImageIO

func fail(_ why: String) -> Never {
    FileHandle.standardError.write((why + "\n").data(using: .utf8)!)
    exit(1)
}

let args = CommandLine.arguments
guard args.count == 3 else { fail("usage: frames2mp4 <list> <out.mp4>") }
guard let list = try? String(contentsOfFile: args[1], encoding: .utf8) else { fail("can't read \(args[1])") }
var frames: [(at: Int64, path: String)] = []
var end: Int64 = 0
for line in list.split(separator: "\n") {
    let parts = line.split(separator: "\t", maxSplits: 1, omittingEmptySubsequences: false)
    guard let at = Int64(parts[0]) else { continue }
    if parts.count < 2 || parts[1].isEmpty { end = at } else { frames.append((at, String(parts[1]))) }
}

func load(_ path: String) -> CGImage? {
    guard let source = CGImageSourceCreateWithURL(URL(fileURLWithPath: path) as CFURL, nil) else { return nil }
    return CGImageSourceCreateImageAtIndex(source, 0, nil)
}

guard let first = frames.first, let image = load(first.path) else { fail("no frames") }
let width = image.width & ~1
let height = image.height & ~1
let out = URL(fileURLWithPath: args[2])
try? FileManager.default.removeItem(at: out)
guard let writer = try? AVAssetWriter(outputURL: out, fileType: .mp4) else { fail("can't write \(args[2])") }
let input = AVAssetWriterInput(mediaType: .video, outputSettings: [AVVideoCodecKey: AVVideoCodecType.h264, AVVideoWidthKey: width, AVVideoHeightKey: height])
input.expectsMediaDataInRealTime = false
let adaptor = AVAssetWriterInputPixelBufferAdaptor(assetWriterInput: input, sourcePixelBufferAttributes: [
    kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32ARGB,
    kCVPixelBufferWidthKey as String: width,
    kCVPixelBufferHeightKey as String: height,
])
writer.add(input)
guard writer.startWriting() else { fail(writer.error?.localizedDescription ?? "can't start writing") }
writer.startSession(atSourceTime: .zero)

func append(_ image: CGImage, at ms: Int64) {
    var made: CVPixelBuffer?
    guard let pool = adaptor.pixelBufferPool, CVPixelBufferPoolCreatePixelBuffer(nil, pool, &made) == kCVReturnSuccess, let buffer = made else { return }
    CVPixelBufferLockBaseAddress(buffer, [])
    if let context = CGContext(data: CVPixelBufferGetBaseAddress(buffer), width: width, height: height, bitsPerComponent: 8, bytesPerRow: CVPixelBufferGetBytesPerRow(buffer), space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.noneSkipFirst.rawValue) {
        context.draw(image, in: CGRect(x: 0, y: 0, width: width, height: height))
    }
    CVPixelBufferUnlockBaseAddress(buffer, [])
    while !input.isReadyForMoreMediaData { usleep(2000) }
    adaptor.append(buffer, withPresentationTime: CMTime(value: ms, timescale: 1000))
}

var last: CGImage?
for frame in frames {
    guard let image = load(frame.path) else { continue }
    append(image, at: frame.at)
    last = image
}
// The last picture held to the end, and the video ending there.
let lastAt = frames.last?.at ?? 0
if let last, end > lastAt + 1 { append(last, at: end - 1) }
input.markAsFinished()
writer.endSession(atSourceTime: CMTime(value: max(end, lastAt + 1), timescale: 1000))
let done = DispatchSemaphore(value: 0)
writer.finishWriting { done.signal() }
done.wait()
if writer.status != .completed { fail(writer.error?.localizedDescription ?? "couldn't finish \(args[2])") }
