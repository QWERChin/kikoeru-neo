import XCTest

/// Ends the current phase. The next phase still runs.
struct CheckError: Error, CustomStringConvertible {
    let description: String
    init(_ description: String) { self.description = description }
}

/// Holds a value written by a callback on another queue and read after a semaphore.
final class Box<Value>: @unchecked Sendable {
    var value: Value
    init(_ value: Value) { self.value = value }
}

/// What the workflow passes through `TEST_RUNNER_` variables, which
/// xcodebuild hands to the test runner without the prefix.
struct Settings {
    let baseURL: URL
    let controlURL: URL
    let user: String
    let password: String
    let workID: Int
    let outDir: URL?
    /// Bundle IDs that might run a home screen web app, read from the runtime.
    let webAppIDs: [String]

    static func fromEnvironment() throws -> Settings {
        let env = ProcessInfo.processInfo.environment
        func need(_ key: String) throws -> String {
            guard let value = env[key], !value.isEmpty else { throw CheckError("\(key) is not set") }
            return value
        }
        let base = try need("IOS_BASE_URL")
        let control = try need("IOS_CONTROL_URL")
        let work = try need("IOS_WORK_ID")
        guard let baseURL = URL(string: base), let controlURL = URL(string: control), let workID = Int(work) else {
            throw CheckError("IOS_BASE_URL, IOS_CONTROL_URL or IOS_WORK_ID is malformed")
        }
        let out = env["IOS_OUT_DIR"] ?? ""
        return Settings(
            baseURL: baseURL,
            controlURL: controlURL,
            user: try need("IOS_USER"),
            password: try need("IOS_PASSWORD"),
            workID: workID,
            outDir: out.isEmpty ? nil : URL(fileURLWithPath: out, isDirectory: true),
            webAppIDs: (env["IOS_WEBAPP_IDS"] ?? "").split(separator: ",").map(String.init)
        )
    }
}

/// Every check and observation, rewritten to report.json after each one so
/// a crash mid-run still leaves what happened before it.
final class Report {
    struct Entry: Codable {
        let phase: String
        let check: String
        let result: String
        let detail: String
        let at: String
    }

    private(set) var entries: [Entry] = []
    private let file: URL?
    private let clock = ISO8601DateFormatter()

    init(outDir: URL?) {
        if let outDir {
            try? FileManager.default.createDirectory(at: outDir, withIntermediateDirectories: true)
        }
        file = outDir?.appendingPathComponent("report.json")
    }

    func record(_ phase: String, _ check: String, _ result: String, _ detail: String) {
        entries.append(Entry(phase: phase, check: check, result: result, detail: detail, at: clock.string(from: Date())))
        print("[ios-check] \(phase) | \(result) | \(check) | \(detail)")
        guard let file else { return }
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.prettyPrinted, .withoutEscapingSlashes]
        if let data = try? encoder.encode(entries) {
            try? data.write(to: file)
        }
    }

    var failures: Int { entries.filter { $0.result == "fail" }.count }
}

/// The kikoeru-neo API, called the way the page would. Synchronous, because
/// the UI test drives the phone step by step anyway.
final class API {
    struct Response {
        let status: Int
        let json: Any?
        let error: String?
    }

    struct Progress: CustomStringConvertible {
        let trackID: Int?
        let positionMs: Int
        var description: String { "track \(trackID.map(String.init) ?? "none") at \(positionMs) ms" }
    }

    private let base: URL
    private var token: String?
    private let session = URLSession(configuration: .ephemeral)

    init(base: URL) { self.base = base }

    func call(_ method: String, _ path: String, body: [String: Any]? = nil, timeout: TimeInterval = 10) -> Response {
        guard let url = URL(string: path, relativeTo: base) else {
            return Response(status: 0, json: nil, error: "bad path \(path)")
        }
        var request = URLRequest(url: url, timeoutInterval: timeout)
        request.httpMethod = method
        if let token { request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization") }
        if let body {
            request.setValue("application/json", forHTTPHeaderField: "Content-Type")
            request.httpBody = try? JSONSerialization.data(withJSONObject: body)
        }
        let box = Box(Response(status: 0, json: nil, error: "no answer within \(timeout) s"))
        let done = DispatchSemaphore(value: 0)
        session.dataTask(with: request) { data, response, error in
            let status = (response as? HTTPURLResponse)?.statusCode ?? 0
            let json = data.flatMap { try? JSONSerialization.jsonObject(with: $0) }
            box.value = Response(status: status, json: json, error: error?.localizedDescription)
            done.signal()
        }.resume()
        _ = done.wait(timeout: .now() + timeout + 2)
        return box.value
    }

    func login(user: String, password: String) -> Bool {
        let response = call("POST", "/api/auth/login", body: ["name": user, "password": password])
        token = (response.json as? [String: Any])?["token"] as? String
        return token != nil
    }

    /// Track IDs by path within the work.
    func tracks(work: Int) -> [String: Int] {
        let response = call("GET", "/api/works/\(work)")
        let tracks = (response.json as? [String: Any])?["tracks"] as? [[String: Any]] ?? []
        var ids: [String: Int] = [:]
        for track in tracks {
            if let path = track["path"] as? String, let id = track["id"] as? Int { ids[path] = id }
        }
        return ids
    }

    /// The saved position, or nil when the request failed. A work without
    /// progress gives a position of 0 on no track.
    func progress(work: Int) -> Progress? {
        let response = call("GET", "/api/works/\(work)")
        guard response.status == 200, let detail = response.json as? [String: Any] else { return nil }
        guard let progress = detail["progress"] as? [String: Any] else { return Progress(trackID: nil, positionMs: 0) }
        return Progress(trackID: progress["track_id"] as? Int, positionMs: progress["position_ms"] as? Int ?? 0)
    }

    func clearProgress(work: Int) -> Bool {
        call("DELETE", "/api/me/progress/\(work)").status == 204
    }
}
