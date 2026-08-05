import AppKit
import CoreGraphics
import Foundation

guard CommandLine.arguments.count == 5,
      let startX = Double(CommandLine.arguments[1]),
      let startY = Double(CommandLine.arguments[2]),
      let endX = Double(CommandLine.arguments[3]),
      let endY = Double(CommandLine.arguments[4]) else {
    fputs("usage: macos-native-drag.swift <start-x> <start-y> <end-x> <end-y>\n", stderr)
    exit(2)
}

guard CGPreflightPostEventAccess() else {
    fputs("macOS Accessibility input permission is not granted for the Swift test process\n", stderr)
    exit(4)
}

let app = NSWorkspace.shared.runningApplications.first {
    $0.localizedName == "ShellX Cut" || $0.executableURL?.lastPathComponent == "shellx-cut"
}
_ = app?.activate(options: [.activateAllWindows])
usleep(400_000)

func post(_ type: CGEventType, at point: CGPoint) {
    guard let event = CGEvent(
        mouseEventSource: nil,
        mouseType: type,
        mouseCursorPosition: point,
        mouseButton: .left
    ) else {
        fputs("could not create mouse event\n", stderr)
        exit(3)
    }
    event.post(tap: .cghidEventTap)
}

let start = CGPoint(x: startX, y: startY)
let end = CGPoint(x: endX, y: endY)
post(.mouseMoved, at: start)
usleep(120_000)
post(.leftMouseDown, at: start)
usleep(120_000)

for step in 1...18 {
    let fraction = CGFloat(step) / 18
    let point = CGPoint(
        x: start.x + (end.x - start.x) * fraction,
        y: start.y + (end.y - start.y) * fraction
    )
    post(.leftMouseDragged, at: point)
    usleep(35_000)
}

usleep(180_000)
post(.leftMouseUp, at: end)
