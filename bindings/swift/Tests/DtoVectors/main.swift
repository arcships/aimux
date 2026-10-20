import Foundation

@main
struct DtoVectors {
    private struct Entry: Decodable {
        let type: String
        let file: String
    }
    private struct Record: Encodable {
        let file: String
        let ok: Bool
        var value: JSONValue? = nil
        var error: String? = nil

        enum CodingKeys: String, CodingKey { case file, ok, value, error }
        func encode(to encoder: any Encoder) throws {
            var c = encoder.container(keyedBy: CodingKeys.self)
            try c.encode(file, forKey: .file)
            try c.encode(ok, forKey: .ok)
            if ok { try c.encode(value ?? .null, forKey: .value) }
            else { try c.encode(error, forKey: .error) }
        }
    }
    static func main() throws {
        let root = URL(fileURLWithPath: CommandLine.arguments.dropFirst().first ?? "contract-tests/dto", isDirectory: true)
        let decoder = JSONDecoder()
        let encoder = JSONEncoder()
        let entries = try decoder.decode([Entry].self, from: Data(contentsOf: root.appendingPathComponent("index.json")))
        for entry in entries {
            let record: Record
            do {
                let data = try Data(contentsOf: root.appendingPathComponent(entry.file))
                let encoded = try DtoVectorTypes.roundTrip(entry.type, data: data)
                record = Record(file: entry.file, ok: true, value: try decoder.decode(JSONValue.self, from: encoded))
            } catch {
                record = Record(file: entry.file, ok: false, error: String(describing: error))
            }
            print(String(decoding: try encoder.encode(record), as: UTF8.self))
        }
    }
}
