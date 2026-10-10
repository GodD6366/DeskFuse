import Foundation
import Darwin

struct MemoryProcessSample: Codable, Sendable {
    let pid: Int32
    let parentPID: Int32
    let rssKB: Int64
    let virtualSizeKB: Int64
    let cpuPercent: Double
    let command: String
    let physicalFootprintKB: Int64?
    let physicalFootprintPeakKB: Int64?
    let linkedFrameworks: [String]
    let footprintSummary: String?
}

struct MemoryDiagnosticsReport: Codable, Sendable {
    let timestampSeconds: Double
    let subsystem: String
    let appPID: Int32
    let app: MemoryProcessSample?
    let children: [MemoryProcessSample]
    let ocr: ClipboardOCRDiagnostics
    let note: String
}

enum MemoryDiagnostics {
    static func capture(ocr: ClipboardOCRDiagnostics) async -> Data {
        let appPID = ProcessInfo.processInfo.processIdentifier
        return await Task.detached(priority: .utility) {
            let rows = processRows()
            let app = rows.first(where: { $0.pid == appPID }).map(sample)
            let children = rows
                .filter { $0.parentPID == appPID && !isDiagnosticCommand($0.command) }
                .map(sample)
            let report = MemoryDiagnosticsReport(
                timestampSeconds: Date().timeIntervalSince1970,
                subsystem: "memory",
                appPID: appPID,
                app: app,
                children: children,
                ocr: ocr,
                note: "RSS/VSZ come from ps; physical footprint and peak come from footprint when task inspection is permitted. footprintSummary is intentionally bounded to the first 40 lines."
            )
            let encoder = JSONEncoder()
            encoder.outputFormatting = [.sortedKeys]
            var line = (try? encoder.encode(report)) ?? Data("{}".utf8)
            line.append(0x0A)
            return line
        }.value
    }

    private struct ProcessRow {
        let pid: Int32
        let parentPID: Int32
        let rssKB: Int64
        let virtualSizeKB: Int64
        let cpuPercent: Double
        let command: String
    }

    private static func processRows() -> [ProcessRow] {
        guard let output = run("/bin/ps", arguments: ["-axo", "pid=,ppid=,rss=,vsz=,%cpu=,command="]) else {
            return []
        }
        return output.split(whereSeparator: \.isNewline).compactMap { line in
            let fields = line.split(maxSplits: 5, whereSeparator: { $0 == " " || $0 == "\t" })
            guard fields.count == 6,
                  let pid = Int32(fields[0]),
                  let parentPID = Int32(fields[1]),
                  let rssKB = Int64(fields[2]),
                  let virtualSizeKB = Int64(fields[3]),
                  let cpuPercent = Double(fields[4]) else { return nil }
            return ProcessRow(
                pid: pid,
                parentPID: parentPID,
                rssKB: rssKB,
                virtualSizeKB: virtualSizeKB,
                cpuPercent: cpuPercent,
                command: String(fields[5])
            )
        }
    }

    private static func sample(_ row: ProcessRow) -> MemoryProcessSample {
        let executable = row.command.split(separator: " ", maxSplits: 1).first.map(String.init) ?? row.command
        let parsed = run("/usr/bin/footprint", arguments: ["-p", String(row.pid)]).map(parseFootprint)
        return MemoryProcessSample(
                pid: row.pid,
                parentPID: row.parentPID,
                rssKB: row.rssKB,
                virtualSizeKB: row.virtualSizeKB,
                cpuPercent: row.cpuPercent,
                command: row.command,
                physicalFootprintKB: parsed?.footprintKB,
                physicalFootprintPeakKB: parsed?.peakKB,
                linkedFrameworks: linkedFrameworks(executable),
                footprintSummary: parsed?.summary
        )
    }

    private static func parseFootprint(_ output: String) -> (footprintKB: Int64?, peakKB: Int64?, summary: String) {
        func kilobytes(_ value: Substring, _ unit: Substring) -> Int64? {
            guard let number = Double(value) else { return nil }
            switch unit.uppercased() {
            case "KB": return Int64(number)
            case "MB": return Int64(number * 1024)
            case "GB": return Int64(number * 1024 * 1024)
            default: return nil
            }
        }
        func valueAfter(_ marker: String, in line: Substring) -> Int64? {
            guard let range = line.range(of: marker) else { return nil }
            let fields = line[range.upperBound...].split(whereSeparator: { $0 == " " || $0 == "\t" })
            guard fields.count >= 2 else { return nil }
            return kilobytes(fields[0], fields[1])
        }
        let lines = output.split(whereSeparator: \.isNewline)
        let footprintKB = lines.first(where: { $0.contains("Footprint:") })
            .flatMap { valueAfter("Footprint:", in: $0) }
        let peakKB = lines.first(where: { $0.contains("phys_footprint_peak:") })
            .flatMap { valueAfter("phys_footprint_peak:", in: $0) }
        let summary = lines.prefix(40).joined(separator: "\n")
        return (footprintKB, peakKB, summary)
    }

    private static func linkedFrameworks(_ executable: String) -> [String] {
        guard executable.hasPrefix("/"), let output = run("/usr/bin/otool", arguments: ["-L", executable]) else {
            return []
        }
        return output.split(whereSeparator: \.isNewline)
            .dropFirst()
            .map { String($0).trimmingCharacters(in: .whitespacesAndNewlines) }
            .filter { $0.contains("/System/Library/Frameworks/") }
    }

    private static func isDiagnosticCommand(_ command: String) -> Bool {
        let executable = command.split(separator: " ", maxSplits: 1).first.map(String.init) ?? command
        return executable == "/bin/ps" || executable == "/usr/bin/footprint" || executable == "/usr/bin/otool"
    }

    private static func run(_ executable: String, arguments: [String]) -> String? {
        let process = Process()
        let outputURL = FileManager.default.temporaryDirectory
            .appendingPathComponent("deskfuse-memory-\(UUID().uuidString).txt")
        guard FileManager.default.createFile(atPath: outputURL.path, contents: nil),
              let output = try? FileHandle(forWritingTo: outputURL) else { return nil }
        let completion = DispatchSemaphore(value: 0)
        process.executableURL = URL(fileURLWithPath: executable)
        process.arguments = arguments
        process.standardOutput = output
        process.standardError = FileHandle.nullDevice
        process.terminationHandler = { _ in completion.signal() }
        do {
            try process.run()
        } catch {
            try? output.close()
            try? FileManager.default.removeItem(at: outputURL)
            return nil
        }
        let completed = completion.wait(timeout: .now() + 2) == .success
        if !completed {
            process.terminate()
            if completion.wait(timeout: .now() + 0.5) == .timedOut {
                kill(process.processIdentifier, SIGKILL)
                _ = completion.wait(timeout: .now() + 0.5)
            }
        }
        try? output.close()
        defer { try? FileManager.default.removeItem(at: outputURL) }
        guard completed, process.terminationStatus == 0,
              let data = try? Data(contentsOf: outputURL) else { return nil }
        return String(data: data, encoding: .utf8)
    }
}
