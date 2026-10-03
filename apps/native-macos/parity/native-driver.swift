// Native macOS interaction adapter for the experimental GPUI app only.
// OS events and screenshots are supporting evidence, not self-certifying scenario passes.
import Cocoa
import ApplicationServices

func fail(_ message: String) -> Never {
    FileHandle.standardError.write(Data((message + "\n").utf8))
    exit(1)
}

func attribute(_ element: AXUIElement, _ key: String) -> CFTypeRef? {
    var value: CFTypeRef?
    guard AXUIElementCopyAttributeValue(element, key as CFString, &value) == .success else { return nil }
    return value
}

func tree(_ element: AXUIElement, depth: Int = 0) -> [String: Any] {
    if depth > 16 { return ["truncated": true] }
    var result: [String: Any] = [:]
    for (key, name) in [(kAXRoleAttribute, "role"), (kAXTitleAttribute, "title"), (kAXDescriptionAttribute, "description"), (kAXValueAttribute, "value")] {
        if let value = attribute(element, key) as? String { result[name] = value }
    }
    result["children"] = (attribute(element, kAXChildrenAttribute) as? [AXUIElement] ?? []).map { tree($0, depth: depth + 1) }
    return result
}

func output(_ value: [String: Any]) {
    guard JSONSerialization.isValidJSONObject(value), let data = try? JSONSerialization.data(withJSONObject: value, options: [.sortedKeys]) else { fail("Could not encode observation") }
    FileHandle.standardOutput.write(data)
    FileHandle.standardOutput.write(Data("\n".utf8))
}

let args = CommandLine.arguments
if args.count < 3 { fail("Usage: swift native-driver.swift <owned-native-pid> <inspect|click|type|key|capture|close> [arguments]") }
guard let pid = pid_t(args[1]), let app = NSRunningApplication(processIdentifier: pid),
      app.executableURL?.lastPathComponent == "t3code-native-macos" else { fail("Target is not the experimental native T3 client") }
guard AXIsProcessTrusted() else { fail("Grant Accessibility to the execution host in macOS settings") }
let command = args[2]
let screenLocked = (CGSessionCopyCurrentDictionary() as? [String:Any])?["CGSSessionScreenIsLocked"] as? Bool ?? false
if screenLocked {
    if command == "inspect" {
        output(["scope":"native accessibility observation", "pid":pid, "screen_locked":true, "capture_authorized":CGPreflightScreenCaptureAccess()])
        exit(0)
    }
    fail("macOS screen is locked; unlock the Mac before native verification. No input or capture was attempted.")
}
let ax = AXUIElementCreateApplication(pid)
let windows = attribute(ax, kAXWindowsAttribute) as? [AXUIElement] ?? []
guard let window = windows.first else { fail("Native window is unavailable") }
if command == "inspect" {
    output(["scope": "native accessibility observation", "pid": pid, "capture_authorized": CGPreflightScreenCaptureAccess(), "screen_locked": false, "tree": tree(ax)])
    exit(0)
}

// Raising the owned window via Accessibility works for the unbundled development executable.
let front = AXUIElementSetAttributeValue(ax, kAXFrontmostAttribute as CFString, kCFBooleanTrue)
let raised = AXUIElementPerformAction(window, kAXRaiseAction as CFString)
guard front == .success && raised == .success else { fail("Could not raise the owned native window") }
let deadline = Date().addingTimeInterval(3)
while NSWorkspace.shared.frontmostApplication?.processIdentifier != pid && Date() < deadline {
    RunLoop.current.run(until: Date().addingTimeInterval(0.02))
}
guard NSWorkspace.shared.frontmostApplication?.processIdentifier == pid else { fail("Native app is not frontmost; no input was sent") }

switch command {
case "close":
    guard let button = attribute(window,kAXCloseButtonAttribute), CFGetTypeID(button) == AXUIElementGetTypeID(),
          AXUIElementPerformAction(button as! AXUIElement,kAXPressAction as CFString) == .success else { fail("Could not close owned window") }
    output(["scope":"native window lifecycle dispatch", "action":"close", "pid":pid])
case "click":
    guard args.count == 5, let x = Double(args[3]), let y = Double(args[4]), x.isFinite, y.isFinite else { fail("click requires window-relative x y") }
    guard let position = attribute(window, kAXPositionAttribute), let dimensions = attribute(window, kAXSizeAttribute) else { fail("Window geometry unavailable") }
    var origin = CGPoint.zero
    var size = CGSize.zero
    guard CFGetTypeID(position) == AXValueGetTypeID(), CFGetTypeID(dimensions) == AXValueGetTypeID(),
          AXValueGetValue(position as! AXValue, .cgPoint, &origin), AXValueGetValue(dimensions as! AXValue, .cgSize, &size),
          x >= 0, y >= 0, x < size.width, y < size.height else { fail("Click is outside the owned window") }
    let point = CGPoint(x: origin.x + x, y: origin.y + y)
    for kind in [CGEventType.leftMouseDown, .leftMouseUp] {
        guard let event = CGEvent(mouseEventSource: nil, mouseType: kind, mouseCursorPosition: point, mouseButton: .left) else { fail("Could not create mouse event") }
        event.post(tap: .cghidEventTap)
    }
    output(["scope": "native OS input dispatch", "action": "click", "x": x, "y": y, "pid": pid])
case "type":
    // Read text from stdin to keep user input off argv. Dispatch through the native IME input path.
    guard let text = String(data: FileHandle.standardInput.readDataToEndOfFile(), encoding: .utf8), !text.isEmpty else { fail("type requires UTF-8 text on stdin") }
    let utf16 = Array(text.utf16)
    guard let down = CGEvent(keyboardEventSource: nil, virtualKey: 0, keyDown: true), let up = CGEvent(keyboardEventSource: nil, virtualKey: 0, keyDown: false) else { fail("Could not create keyboard input") }
    utf16.withUnsafeBufferPointer { buffer in
        down.keyboardSetUnicodeString(stringLength: utf16.count, unicodeString: buffer.baseAddress!)
        up.keyboardSetUnicodeString(stringLength: utf16.count, unicodeString: buffer.baseAddress!)
    }
    down.post(tap: .cghidEventTap)
    up.post(tap: .cghidEventTap)
    output(["scope": "native OS input dispatch", "action": "type", "utf16_length": utf16.count, "pid": pid])
case "key":
    guard args.count == 4 else { fail("key requires cmd-a, cmd-c, cmd-v, enter, tab or escape") }
    let keys: [String: (CGKeyCode, CGEventFlags)] = ["cmd-a":(0,.maskCommand), "cmd-c":(8,.maskCommand), "cmd-v":(9,.maskCommand), "enter":(36,[]), "tab":(48,[]), "escape":(53,[])]
    guard let (code, flags) = keys[args[3]] else { fail("Unsupported key") }
    for isDown in [true, false] {
        guard let event = CGEvent(keyboardEventSource:nil,virtualKey:code,keyDown:isDown) else { fail("Could not create key event") }
        event.flags = flags
        event.post(tap:.cghidEventTap)
    }
    output(["scope":"native OS input dispatch", "action":"key", "key":args[3], "pid":pid])
case "capture":
    guard args.count == 4, CGPreflightScreenCaptureAccess() else { fail("capture requires an external output path and Screen Recording permission") }
    let destination = URL(fileURLWithPath:args[3]).standardizedFileURL
    guard destination.path.hasPrefix("/"), destination.pathExtension == "png", !FileManager.default.fileExists(atPath:destination.path) else { fail("Use a new absolute PNG evidence path") }
    let rows = CGWindowListCopyWindowInfo(.optionOnScreenOnly,kCGNullWindowID) as? [[String:Any]] ?? []
    guard let row = rows.first(where: { ($0[kCGWindowOwnerPID as String] as? Int) == Int(pid) && ($0[kCGWindowLayer as String] as? Int) == 0 }), let number = row[kCGWindowNumber as String] as? Int else { fail("Owned visible window not found") }
    let process = Process()
    process.executableURL = URL(fileURLWithPath:"/usr/sbin/screencapture")
    process.arguments = ["-x","-o","-l",String(number),destination.path]
    do { try process.run() } catch { fail("Could not start capture") }
    process.waitUntilExit()
    guard process.terminationStatus == 0, FileManager.default.fileExists(atPath:destination.path) else { fail("Window capture failed") }
    output(["scope":"native window observation", "artifact":destination.path, "window_id":number, "pid":pid])
default: fail("Unknown command")
}
