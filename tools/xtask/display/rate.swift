// The display's rate for `cargo xtask measure frame-pace` (tools/xtask/README.md,
// "frame-pace") on macOS: the main display's mode's exact refresh rate, the display link's
// nominal period as a rate, and the screen's greatest frames a second, one `name value` line
// each.
//
//   swift tools/xtask/display/rate.swift
//
// The mode's rate is 0 for a display of variable rate; the display link's nominal period is
// then the figure. The screen's greatest frames a second is the whole number MoltenVK reports
// as a swapchain's refresh, and is printed beside the others, never in their place. A line is
// left out where its source gives nothing.

import AppKit
import CoreGraphics
import CoreVideo

let display = CGMainDisplayID()
if let mode = CGDisplayCopyDisplayMode(display) {
    print(String(format: "mode_hz %.6f", mode.refreshRate))
}
var link: CVDisplayLink?
if CVDisplayLinkCreateWithCGDisplay(display, &link) == kCVReturnSuccess, let link {
    let period = CVDisplayLinkGetNominalOutputVideoRefreshPeriod(link)
    let indefinite = period.flags & Int32(CVTimeFlags.isIndefinite.rawValue) != 0
    if !indefinite && period.timeValue > 0 {
        print(String(format: "link_hz %.6f", Double(period.timeScale) / Double(period.timeValue)))
    }
}
if let screen = NSScreen.main {
    print("screen_fps \(screen.maximumFramesPerSecond)")
}
