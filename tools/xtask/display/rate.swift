// The displays' rates for `cargo xtask measure frame-pace` (tools/xtask/README.md,
// "frame-pace") on macOS: one line for each active display, its id, `main` for the main
// display, then its mode's exact refresh rate, its display link's nominal period as a rate
// and its screen's greatest frames a second, each as `name value`.
//
//   swift tools/xtask/display/rate.swift
//
// The mode's rate is 0 for a display of variable rate; the display link's nominal period is
// then the figure. MoltenVK reports a swapchain's refresh from the mode's rate of the screen
// its layer is on, falling back to the screen's greatest frames a second only where the mode
// reports 0, and to 60 where that is 0 too; the greatest frames a second is printed beside
// the others, never in their place. A field is left out where its source gives nothing.

import AppKit
import CoreGraphics
import CoreVideo

var count: UInt32 = 0
guard CGGetActiveDisplayList(0, nil, &count) == .success else { exit(1) }
var displays = [CGDirectDisplayID](repeating: 0, count: Int(count))
guard CGGetActiveDisplayList(count, &displays, &count) == .success else { exit(1) }
let main = CGMainDisplayID()
let number = NSDeviceDescriptionKey("NSScreenNumber")
for display in displays.prefix(Int(count)) {
    var line = "display \(display)"
    if display == main {
        line += " main"
    }
    if let mode = CGDisplayCopyDisplayMode(display) {
        line += String(format: " mode_hz %.6f", mode.refreshRate)
    }
    var link: CVDisplayLink?
    if CVDisplayLinkCreateWithCGDisplay(display, &link) == kCVReturnSuccess, let link {
        let period = CVDisplayLinkGetNominalOutputVideoRefreshPeriod(link)
        let indefinite = period.flags & Int32(CVTimeFlags.isIndefinite.rawValue) != 0
        if !indefinite && period.timeValue > 0 {
            line += String(format: " link_hz %.6f", Double(period.timeScale) / Double(period.timeValue))
        }
    }
    let screen = NSScreen.screens.first {
        ($0.deviceDescription[number] as? NSNumber)?.uint32Value == display
    }
    if let screen {
        line += " screen_fps \(screen.maximumFramesPerSecond)"
    }
    print(line)
}
