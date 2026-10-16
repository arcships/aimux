//! Swift Codable declarations and vector dispatch from the normalized model.
use super::Emitter;
use crate::model::{Codec, Field, Kind, Model, Ty};
use std::{collections::BTreeMap, fmt::Write, path::PathBuf};

pub struct Swift;

fn quoted(value: &str) -> String {
    let mut result = String::from("\"");
    for ch in value.chars() {
        match ch {
            '"' => result.push_str("\\\""),
            '\\' => result.push_str("\\\\"),
            '\n' => result.push_str("\\n"),
            '\r' => result.push_str("\\r"),
            '\t' => result.push_str("\\t"),
            ch if ch.is_control() => write!(result, "\\u{{{:x}}}", u32::from(ch)).unwrap(),
            ch => result.push(ch),
        }
    }
    result.push('"');
    result
}

fn ident(value: &str) -> String {
    if [
        "Any",
        "Self",
        "Type",
        "Protocol",
        "associatedtype",
        "as",
        "break",
        "case",
        "catch",
        "class",
        "continue",
        "default",
        "defer",
        "deinit",
        "do",
        "else",
        "enum",
        "extension",
        "fallthrough",
        "false",
        "fileprivate",
        "for",
        "func",
        "guard",
        "if",
        "import",
        "in",
        "init",
        "inout",
        "internal",
        "is",
        "let",
        "nil",
        "operator",
        "private",
        "precedencegroup",
        "protocol",
        "public",
        "repeat",
        "rethrows",
        "return",
        "self",
        "static",
        "struct",
        "subscript",
        "super",
        "switch",
        "throw",
        "throws",
        "true",
        "try",
        "typealias",
        "var",
        "where",
        "while",
    ]
    .contains(&value)
    {
        format!("`{value}`")
    } else {
        value.to_owned()
    }
}

fn camel(value: &str) -> String {
    let mut result = String::new();
    let mut upper = false;
    for ch in value.chars() {
        if !ch.is_ascii_alphanumeric() {
            upper = true;
        } else if result.is_empty() {
            result.push(ch.to_ascii_lowercase());
            upper = false;
        } else {
            result.push(if upper { ch.to_ascii_uppercase() } else { ch });
            upper = false;
        }
    }
    if result.is_empty() || result.starts_with(|ch: char| ch.is_ascii_digit()) {
        result.insert_str(0, "value");
    }
    let chars: Vec<_> = result.chars().collect();
    let run = 1 + chars
        .iter()
        .skip(1)
        .take_while(|ch| ch.is_ascii_uppercase())
        .count();
    let prefix = if run < chars.len() { run - 1 } else { run };
    result = chars
        .iter()
        .enumerate()
        .map(|(i, &ch)| {
            if i < prefix {
                ch.to_ascii_lowercase()
            } else {
                ch
            }
        })
        .collect();
    ident(&result)
}

fn ty(value: &Ty) -> String {
    match value {
        Ty::Ref(name) => ident(name),
        Ty::String => "String".into(),
        Ty::Bool => "Bool".into(),
        Ty::Int { unsigned, bits } => format!("{}Int{bits}", if *unsigned { "U" } else { "" }),
        Ty::Float => "Double".into(),
        Ty::Array(inner) => format!("[{}]", ty(inner)),
        Ty::Map(inner) => format!("[String: {}]", ty(inner)),
        Ty::Json => "JSONValue".into(),
        Ty::Nullable(inner) => format!("{}?", ty(inner)),
        Ty::Codec(Codec::U64String, _) => "WireUInt64".into(),
        Ty::Codec(Codec::Presence, inner) => format!("FieldPresence<{}>", ty(nonnull(inner))),
        Ty::Codec(_, inner) => ty(inner),
    }
}

fn nonnull(value: &Ty) -> &Ty {
    match value {
        Ty::Nullable(inner) => nonnull(inner),
        _ => value,
    }
}

fn branch_names(branches: &[Ty]) -> Vec<String> {
    let mut used = BTreeMap::<String, usize>::new();
    branches
        .iter()
        .map(|branch| {
            let base = match branch {
                Ty::Ref(name) => name.as_str(),
                Ty::String => "string",
                Ty::Bool => "bool",
                Ty::Int { .. } => "integer",
                Ty::Float => "number",
                Ty::Array(_) => "array",
                Ty::Map(_) => "map",
                Ty::Json => "json",
                Ty::Nullable(_) => "nullable",
                Ty::Codec(_, _) => "coded",
            };
            let name = camel(base);
            let count = used.entry(name.clone()).or_default();
            *count += 1;
            if *count == 1 {
                name
            } else {
                ident(&format!("{}{count}", name.trim_matches('`')))
            }
        })
        .collect()
}

fn presence(field: &Field) -> bool {
    matches!(field.ty, Ty::Codec(Codec::Presence, _))
}

fn optional(field: &Field) -> bool {
    !field.required && !presence(field)
}

fn field_ty(field: &Field) -> String {
    if optional(field) {
        format!("{}?", ty(nonnull(&field.ty)))
    } else {
        ty(&field.ty)
    }
}

fn docs(out: &mut String, doc: Option<&str>, indent: &str) {
    if let Some(doc) = doc {
        for line in doc.lines() {
            let line = line.trim_end();
            if line.is_empty() {
                writeln!(out, "{indent}///").unwrap();
            } else {
                writeln!(out, "{indent}/// {line}").unwrap();
            }
        }
    }
}

fn keys(out: &mut String, fields: &[Field], tag: Option<&str>) {
    let mut names: Vec<&str> = fields.iter().map(|f| f.json_name.as_str()).collect();
    names.extend(tag);
    names.sort_unstable();
    names.dedup();
    if names.is_empty() {
        return;
    }
    out.push_str("    private enum CodingKeys: String, CodingKey {\n");
    for name in names {
        writeln!(out, "        case {} = {}", camel(name), quoted(name)).unwrap();
    }
    out.push_str("    }\n");
}

fn decode(field: &Field) -> String {
    let key = camel(&field.json_name);
    if presence(field) {
        let Ty::Codec(_, inner) = &field.ty else {
            unreachable!()
        };
        format!(
            "try c.decodePresence({}.self, forKey: .{key})",
            ty(nonnull(inner))
        )
    } else if optional(field) {
        format!(
            "try c.decodeIfPresent({}.self, forKey: .{key})",
            ty(nonnull(&field.ty))
        )
    } else {
        format!("try c.decode({}.self, forKey: .{key})", ty(&field.ty))
    }
}

fn encode(out: &mut String, field: &Field, value: &str, indent: &str) {
    let method = if presence(field) {
        "encodePresence"
    } else if optional(field) {
        "encodeIfPresent"
    } else {
        "encode"
    };
    writeln!(
        out,
        "{indent}try c.{method}({value}, forKey: .{})",
        camel(&field.json_name)
    )
    .unwrap();
}

fn structure(out: &mut String, name: &str, fields: &[Field]) {
    writeln!(
        out,
        "public struct {}: Codable, Equatable, Sendable {{",
        ident(name)
    )
    .unwrap();
    for field in fields {
        docs(out, field.doc.as_deref(), "    ");
        writeln!(
            out,
            "    public var {}: {}",
            camel(&field.json_name),
            field_ty(field)
        )
        .unwrap();
    }
    out.push_str("\n    public init(");
    for (i, field) in fields.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        write!(
            out,
            "{}: {}{}",
            camel(&field.json_name),
            field_ty(field),
            if presence(field) {
                " = .missing"
            } else if optional(field) {
                " = nil"
            } else {
                ""
            }
        )
        .unwrap();
    }
    out.push_str(") {\n");
    for field in fields {
        let key = camel(&field.json_name);
        writeln!(out, "        self.{key} = {key}").unwrap();
    }
    out.push_str("    }\n");
    if !fields.is_empty()
        && fields.iter().all(|field| {
            !presence(field) && !(field.required && matches!(field.ty, Ty::Nullable(_)))
        })
    {
        if fields
            .iter()
            .any(|field| camel(&field.json_name).trim_matches('`') != field.json_name)
        {
            keys(out, fields, None);
        }
        out.push_str("}\n\n");
        return;
    }
    keys(out, fields, None);
    out.push_str("    public init(from decoder: any Decoder) throws {\n");
    if fields.is_empty() {
        out.push_str("        _ = try decoder.container(keyedBy: WireKey.self)\n");
    } else {
        out.push_str("        let c = try decoder.container(keyedBy: CodingKeys.self)\n");
        for field in fields {
            writeln!(
                out,
                "        self.{} = {}",
                camel(&field.json_name),
                decode(field)
            )
            .unwrap();
        }
    }
    out.push_str("    }\n    public func encode(to encoder: any Encoder) throws {\n");
    if fields.is_empty() {
        out.push_str("        _ = encoder.container(keyedBy: WireKey.self)\n");
    } else {
        out.push_str("        var c = encoder.container(keyedBy: CodingKeys.self)\n");
        for field in fields {
            encode(
                out,
                field,
                &format!("self.{}", camel(&field.json_name)),
                "        ",
            );
        }
    }
    out.push_str("    }\n}\n\n");
}

impl Emitter for Swift {
    fn owned(&self) -> &'static [&'static str] {
        &[
            "bindings/swift/generated/Types.swift",
            "bindings/swift/generated/DtoVectorTypes.swift",
        ]
    }

    fn files(&self, model: &Model) -> Vec<(PathBuf, String)> {
        let mut out = SUPPORT.to_owned();
        for named in &model.types {
            docs(&mut out, named.doc.as_deref(), "");
            let name = ident(&named.name);
            match &named.kind {
                Kind::Alias(value) => {
                    writeln!(out, "public typealias {name} = {}\n", ty(value)).unwrap()
                }
                Kind::Struct { fields, .. } => structure(&mut out, &named.name, fields),
                Kind::StringEnum(values) => {
                    writeln!(
                        out,
                        "public enum {name}: String, Codable, Equatable, Sendable {{"
                    )
                    .unwrap();
                    for value in values {
                        writeln!(out, "    case {} = {}", camel(value), quoted(value)).unwrap();
                    }
                    out.push_str("}\n\n");
                }
                Kind::Untagged(branches) => {
                    let names = branch_names(branches);
                    writeln!(
                        out,
                        "public indirect enum {name}: Codable, Equatable, Sendable {{"
                    )
                    .unwrap();
                    for (branch, case) in branches.iter().zip(&names) {
                        writeln!(out, "    case {case}({})", ty(branch)).unwrap();
                    }
                    out.push_str("\n    public init(from decoder: any Decoder) throws {\n");
                    for (branch, case) in branches.iter().zip(&names) {
                        writeln!(out, "        do {{ let value = try decoder.singleValueContainer().decode({}.self); self = .{case}(value); return }} catch {{}}", ty(branch)).unwrap();
                    }
                    writeln!(out, "        throw DecodingError.dataCorrupted(.init(codingPath: decoder.codingPath, debugDescription: {}))", quoted(&format!("No matching {0} branch", named.name))).unwrap();
                    out.push_str("    }\n    public func encode(to encoder: any Encoder) throws {\n        switch self {\n");
                    for case in names {
                        writeln!(
                            out,
                            "        case .{case}(let value): try value.encode(to: encoder)"
                        )
                        .unwrap();
                    }
                    out.push_str("        }\n    }\n}\n\n");
                }
                Kind::Tagged { tag, variants } => {
                    writeln!(
                        out,
                        "public indirect enum {name}: Codable, Equatable, Sendable {{"
                    )
                    .unwrap();
                    for variant in variants {
                        for field in &variant.fields {
                            if let Some(doc) = &field.doc {
                                docs(
                                    &mut out,
                                    Some(&format!("{}: {doc}", field.json_name)),
                                    "    ",
                                );
                            }
                        }
                        write!(out, "    case {}", camel(&variant.name)).unwrap();
                        if !variant.fields.is_empty() {
                            let fields: Vec<_> = variant
                                .fields
                                .iter()
                                .map(|f| format!("{}: {}", camel(&f.json_name), field_ty(f)))
                                .collect();
                            write!(out, "({})", fields.join(", ")).unwrap();
                        }
                        out.push('\n');
                    }
                    let fields: Vec<_> = variants.iter().flat_map(|v| v.fields.iter()).collect();
                    out.push_str("    private enum CodingKeys: String, CodingKey {\n");
                    let mut names: Vec<_> = fields
                        .iter()
                        .map(|f| f.json_name.as_str())
                        .chain([tag.as_str()])
                        .collect();
                    names.sort_unstable();
                    names.dedup();
                    for key in names {
                        writeln!(out, "        case {} = {}", camel(key), quoted(key)).unwrap();
                    }
                    out.push_str("    }\n    public init(from decoder: any Decoder) throws {\n        let c = try decoder.container(keyedBy: CodingKeys.self)\n");
                    writeln!(out, "        let tag = try c.decode(String.self, forKey: .{})\n        switch tag {{", camel(tag)).unwrap();
                    let mut groups = BTreeMap::<&str, Vec<_>>::new();
                    for variant in variants {
                        groups.entry(&variant.tag_value).or_default().push(variant);
                    }
                    for (tag_value, group) in groups {
                        writeln!(out, "        case {}:", quoted(tag_value)).unwrap();
                        for (i, variant) in group.iter().enumerate() {
                            if group.len() > 1 {
                                out.push_str("            do {\n");
                            }
                            let indent = if group.len() > 1 {
                                "                "
                            } else {
                                "            "
                            };
                            write!(out, "{indent}self = .{}", camel(&variant.name)).unwrap();
                            if !variant.fields.is_empty() {
                                let values: Vec<_> = variant
                                    .fields
                                    .iter()
                                    .map(|f| {
                                        format!(
                                            "{}: {}",
                                            camel(&f.json_name).trim_matches('`'),
                                            decode(f)
                                        )
                                    })
                                    .collect();
                                write!(out, "({})", values.join(", ")).unwrap();
                            }
                            out.push('\n');
                            writeln!(out, "{indent}return").unwrap();
                            if group.len() > 1 {
                                if i + 1 == group.len() {
                                    out.push_str("            } catch { throw error }\n");
                                } else {
                                    out.push_str("            } catch {}\n");
                                }
                            }
                        }
                    }
                    writeln!(out, "        default: throw DecodingError.dataCorrupted(.init(codingPath: decoder.codingPath, debugDescription: \"Unknown {} tag \\(tag)\"))", named.name).unwrap();
                    out.push_str("        }\n    }\n    public func encode(to encoder: any Encoder) throws {\n        var c = encoder.container(keyedBy: CodingKeys.self)\n        switch self {\n");
                    for variant in variants {
                        write!(out, "        case .{}", camel(&variant.name)).unwrap();
                        if !variant.fields.is_empty() {
                            let values: Vec<_> = (0..variant.fields.len())
                                .map(|i| format!("let v{i}"))
                                .collect();
                            write!(out, "({})", values.join(", ")).unwrap();
                        }
                        writeln!(
                            out,
                            ":\n            try c.encode({}, forKey: .{})",
                            quoted(&variant.tag_value),
                            camel(tag)
                        )
                        .unwrap();
                        for (i, field) in variant.fields.iter().enumerate() {
                            encode(&mut out, field, &format!("v{i}"), "            ");
                        }
                    }
                    out.push_str("        }\n    }\n}\n\n");
                }
            }
        }
        let mut runner = String::from(
            "// Generated by aimux-codegen. Do not edit.\nimport Foundation\n\nenum DtoVectorTypes {\n    static func roundTrip(_ name: String, data: Data) throws -> Data {\n        let decoder = JSONDecoder()\n        let encoder = JSONEncoder()\n        switch name {\n",
        );
        for named in &model.types {
            writeln!(
                runner,
                "        case {}: return try encoder.encode(decoder.decode({}.self, from: data))",
                quoted(&named.name),
                ident(&named.name)
            )
            .unwrap();
        }
        // The shared model omits unreferenced primitive aliases from declarations.
        for (name, value) in [
            ("AnyValue", "JSONValue"),
            ("null", "Optional<JSONValue>"),
            ("string", "String"),
            ("_uint8_", "[UInt8]"),
        ] {
            if !model.types.iter().any(|named| named.name == name) {
                writeln!(runner, "        case {}: return try encoder.encode(decoder.decode({value}.self, from: data))", quoted(name)).unwrap();
            }
        }
        runner.push_str("        default: throw DecodingError.dataCorrupted(.init(codingPath: [], debugDescription: \"Unknown vector type \\(name)\"))\n        }\n    }\n}\n");
        vec![
            ("bindings/swift/generated/Types.swift".into(), out),
            (
                "bindings/swift/generated/DtoVectorTypes.swift".into(),
                runner,
            ),
        ]
    }
}

const SUPPORT: &str = r#"// Generated by aimux-codegen. Do not edit.
import Foundation

/// JSON numbers retain integral values before considering decimal representation.
public indirect enum JSONValue: Codable, Equatable, Sendable {
    case null
    case bool(Bool)
    case integer(Int64)
    case unsignedInteger(UInt64)
    case number(Decimal)
    case string(String)
    case array([JSONValue])
    case object([String: JSONValue])

    public init(from decoder: any Decoder) throws {
        let c = try decoder.singleValueContainer()
        if c.decodeNil() { self = .null }
        else if let value = try? c.decode(Bool.self) { self = .bool(value) }
        else if let value = try? c.decode(String.self) { self = .string(value) }
        else if let value = try? c.decode(Int64.self) { self = .integer(value) }
        else if let value = try? c.decode(UInt64.self) { self = .unsignedInteger(value) }
        else if let value = try? c.decode(Decimal.self) { self = .number(value) }
        else if let value = try? c.decode([JSONValue].self) { self = .array(value) }
        else { self = .object(try c.decode([String: JSONValue].self)) }
    }
    public func encode(to encoder: any Encoder) throws {
        var c = encoder.singleValueContainer()
        switch self {
        case .null: try c.encodeNil()
        case .bool(let value): try c.encode(value)
        case .integer(let value): try c.encode(value)
        case .unsignedInteger(let value): try c.encode(value)
        case .number(let value): try c.encode(value)
        case .string(let value): try c.encode(value)
        case .array(let value): try c.encode(value)
        case .object(let value): try c.encode(value)
        }
    }
    public subscript(key: String) -> JSONValue? { objectValue?[key] }
    public subscript(index: Int) -> JSONValue? {
        guard let values = arrayValue, values.indices.contains(index) else { return nil }
        return values[index]
    }
    public var stringValue: String? { if case .string(let value) = self { return value }; return nil }
    public var boolValue: Bool? { if case .bool(let value) = self { return value }; return nil }
    public var doubleValue: Double? {
        switch self {
        case .integer(let value): return Double(value)
        case .unsignedInteger(let value): return Double(value)
        case .number(let value): return NSDecimalNumber(decimal: value).doubleValue
        default: return nil
        }
    }
    public var intValue: Int? {
        switch self {
        case .integer(let value): return Int(exactly: value)
        case .unsignedInteger(let value): return Int(exactly: value)
        case .number(let value):
            let number = NSDecimalNumber(decimal: value)
            let result = number.int64Value
            return Decimal(result) == value ? Int(exactly: result) : nil
        default: return nil
        }
    }
    public var arrayValue: [JSONValue]? { if case .array(let value) = self { return value }; return nil }
    public var objectValue: [String: JSONValue]? { if case .object(let value) = self { return value }; return nil }
}

/// The three wire states used only by fields marked with the presence codec.
public enum FieldPresence<Value: Codable & Equatable & Sendable>: Equatable, Sendable {
    case missing
    case null
    case value(Value)
}
extension FieldPresence: Codable {
    public init(from decoder: any Decoder) throws {
        let c = try decoder.singleValueContainer()
        self = c.decodeNil() ? .null : .value(try c.decode(Value.self))
    }
    public func encode(to encoder: any Encoder) throws {
        var c = encoder.singleValueContainer()
        switch self {
        case .missing: throw EncodingError.invalidValue(self, .init(codingPath: encoder.codingPath, debugDescription: "Missing field has no JSON value"))
        case .null: try c.encodeNil()
        case .value(let value): try c.encode(value)
        }
    }
}
private extension KeyedDecodingContainer {
    func decodePresence<T: Codable & Equatable & Sendable>(_ type: T.Type, forKey key: Key) throws -> FieldPresence<T> {
        guard contains(key) else { return .missing }
        return try decode(FieldPresence<T>.self, forKey: key)
    }
}
private extension KeyedEncodingContainer {
    mutating func encodePresence<T: Codable & Equatable & Sendable>(_ value: FieldPresence<T>, forKey key: Key) throws {
        if case .missing = value { return }
        try encode(value, forKey: key)
    }
}

/// A checked UInt64 decimal string; its wire spelling remains unchanged.
public struct WireUInt64: Codable, Equatable, Sendable {
    public let rawValue: String
    public var value: UInt64 { UInt64(rawValue)! }
    public init(_ rawValue: String) throws {
        guard !rawValue.isEmpty, rawValue.utf8.allSatisfy({ (48...57).contains($0) }), UInt64(rawValue) != nil else {
            throw DecodingError.dataCorrupted(.init(codingPath: [], debugDescription: "Invalid UInt64 string"))
        }
        self.rawValue = rawValue
    }
    public init(from decoder: any Decoder) throws {
        let c = try decoder.singleValueContainer()
        let value = try c.decode(String.self)
        do { try self.init(value) }
        catch { throw DecodingError.dataCorruptedError(in: c, debugDescription: "Invalid UInt64 string") }
    }
    public func encode(to encoder: any Encoder) throws {
        var c = encoder.singleValueContainer()
        try c.encode(rawValue)
    }
}
private struct WireKey: CodingKey {
    let stringValue: String
    let intValue: Int? = nil
    init?(stringValue: String) { self.stringValue = stringValue }
    init?(intValue: Int) { return nil }
}

"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::normalize;
    use serde_json::json;

    #[test]
    fn fields_keep_required_null_and_presence_separate() {
        let model = normalize(&json!({"input": {}, "output": {"Record": {
            "type": "object", "properties": {
                "required": {"type": ["string", "null"]},
                "optional": {"type": ["string", "null"]},
                "body": {"x-aimux-codec": "presence"}
            }, "required": ["required"]
        }}}))
        .unwrap();
        let files = Swift.files(&model);
        let source = &files[0].1;
        assert!(source.contains("c.decode(String?.self, forKey: .required)"));
        assert!(source.contains("c.decodeIfPresent(String.self, forKey: .optional)"));
        assert!(source.contains("c.decodePresence(JSONValue.self, forKey: .body)"));
        assert!(source.contains("c.encode(self.required, forKey: .required)"));
        assert!(source.contains("c.encodePresence(self.body, forKey: .body)"));
    }

    #[test]
    fn swift_property_names_keep_the_original_wire_keys() {
        let model = normalize(&json!({"input": {}, "output": {"Record": {
            "type": "object", "properties": {"snake_case": {"type": "string"}}, "required": ["snake_case"]
        }}})).unwrap();
        let files = Swift.files(&model);
        let declaration = files[0].1.split("public struct Record").nth(1).unwrap();
        assert!(declaration.contains("public var snakeCase: String"));
        assert!(declaration.contains("case snakeCase = \"snake_case\""));
        assert!(!declaration.contains("init(from:"));
    }

    #[test]
    fn unions_keep_branch_order_and_nested_discriminators() {
        let model = normalize(&json!({"input": {}, "output": {
            "Choice": {"anyOf": [{"type": "integer"}, {"type": "string"}], "x-aimux-codec": "ordered-untagged"},
            "Event": {"oneOf": [
                {"type": "object", "properties": {"type": {"const": "a"}, "value": {"type": "string"}}, "required": ["type", "value"]},
                {"type": "object", "properties": {"type": {"const": "b"}}, "required": ["type"]}
            ]}
        }})).unwrap();
        let files = Swift.files(&model);
        let source = &files[0].1;
        assert!(
            source.find("case integer(Int64)").unwrap()
                < source.find("case string(String)").unwrap()
        );
        assert!(source.contains("Unknown Event tag \\(tag)"));
        assert!(source.contains("try c.encode(\"a\", forKey: .type)"));
        assert!(
            files[1]
                .1
                .contains("decoder.decode(Event.self, from: data)")
        );
        assert_eq!(files, Swift.files(&model));
    }

    #[test]
    fn full_manifest_covers_vectors_and_is_deterministic() {
        let mut model = normalize(&aimux_ops::manifest::manifest()["typeDefs"]).unwrap();
        model.drop_builtin_aliases();
        let files = Swift.files(&model);
        assert_eq!(files, Swift.files(&model));
        let index: serde_json::Value =
            serde_json::from_str(include_str!("../../../../contract-tests/dto/index.json"))
                .unwrap();
        for vector in index.as_array().unwrap() {
            let name = vector["type"].as_str().unwrap();
            assert!(
                files[1].1.contains(&format!("case {}:", quoted(name))),
                "missing vector type {name}"
            );
        }
    }
}
