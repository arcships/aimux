//! Go DTOs and their JSON codecs, generated solely from the normalized model.
use super::Emitter;
use crate::model::{Codec, Field, Kind, Model, Ty};
use std::{
    fmt::Write,
    io::Write as _,
    path::PathBuf,
    process::{Command, Stdio},
};

pub struct Go;

fn ident(value: &str) -> String {
    let mut result = String::new();
    for part in value
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|s| !s.is_empty())
    {
        let mut chars = part.chars();
        let first = chars.next().unwrap();
        if result.is_empty() && first.is_ascii_digit() {
            result.push('V');
        }
        result.push(first.to_ascii_uppercase());
        result.extend(chars);
    }
    for (word, initialism) in [
        ("Id", "ID"),
        ("Url", "URL"),
        ("Api", "API"),
        ("Abi", "ABI"),
        ("Json", "JSON"),
        ("Http", "HTTP"),
    ] {
        if result.ends_with(word) {
            result.truncate(result.len() - word.len());
            result.push_str(initialism);
        } else if result.starts_with(word)
            && result
                .as_bytes()
                .get(word.len())
                .is_some_and(u8::is_ascii_uppercase)
        {
            result.replace_range(..word.len(), initialism);
        }
    }
    result
}

fn ty(value: &Ty) -> String {
    match value {
        Ty::Ref(name) => ident(name),
        Ty::String => "string".into(),
        Ty::Bool => "bool".into(),
        Ty::Int {
            unsigned: true,
            bits: 8,
        } => "Uint8Number".into(),
        Ty::Int { unsigned, bits } => format!("{}int{bits}", if *unsigned { "u" } else { "" }),
        Ty::Float => "float64".into(),
        Ty::Array(inner) => format!("[]{}", ty(inner)),
        Ty::Map(inner) => format!("map[string]{}", ty(inner)),
        Ty::Json => "json.RawMessage".into(),
        Ty::Nullable(inner) => format!("*{}", ty(inner)),
        Ty::Codec(Codec::U64String, _) => "Uint64String".into(),
        Ty::Codec(_, inner) => ty(inner),
    }
}

fn nullable(value: &Ty, model: &Model) -> bool {
    match value {
        Ty::Nullable(_) | Ty::Json => true,
        Ty::Codec(_, inner) => nullable(inner, model),
        Ty::Ref(name) => match &model.types.iter().find(|t| &t.name == name).unwrap().kind {
            Kind::Alias(inner) => nullable(inner, model),
            _ => false,
        },
        _ => false,
    }
}

fn presence(value: &Ty) -> bool {
    match value {
        Ty::Codec(Codec::Presence, _) => true,
        Ty::Nullable(inner) | Ty::Codec(_, inner) => presence(inner),
        _ => false,
    }
}

fn field_inner(value: &Ty) -> &Ty {
    match value {
        Ty::Nullable(inner) | Ty::Codec(Codec::Presence, inner) => field_inner(inner),
        _ => value,
    }
}

fn optional(field: &Field) -> bool {
    !field.required || presence(&field.ty)
}

fn docs(out: &mut String, doc: Option<&str>) {
    if let Some(doc) = doc {
        for line in doc.lines() {
            writeln!(out, "// {line}").unwrap();
        }
    }
}

fn structure(
    out: &mut String,
    name: &str,
    fields: &[Field],
    tag: Option<(&str, &str)>,
    model: &Model,
) {
    writeln!(out, "type {name} struct {{").unwrap();
    for field in fields {
        docs(out, field.doc.as_deref());
        let field_ty = if optional(field) {
            format!("Field[{}]", ty(field_inner(&field.ty)))
        } else {
            ty(&field.ty)
        };
        writeln!(
            out,
            "{} {} `json:\"{}{}\"`",
            ident(&field.json_name),
            field_ty,
            field.json_name,
            if field.required { "" } else { ",omitempty" }
        )
        .unwrap();
    }
    let object_binding = if fields.is_empty() && tag.is_none() {
        "_"
    } else {
        "object"
    };
    writeln!(out, "}}\nfunc (v *{name}) UnmarshalJSON(data []byte) error {{\n{object_binding}, err := dtoObject(data)\nif err != nil {{ return fmt.Errorf(\"{name}: %w\", err) }}\nvar next {name}").unwrap();
    if let Some((key, value)) = tag {
        writeln!(out, "if err := dtoTag(object, {key:?}, {value:?}); err != nil {{ return fmt.Errorf(\"{name}: %w\", err) }}").unwrap();
    }
    for field in fields {
        writeln!(out, "if err := dtoRead(object, {:?}, &next.{}, {}, {}); err != nil {{ return fmt.Errorf(\"{name}: %w\", err) }}", field.json_name, ident(&field.json_name), field.required, nullable(&field.ty, model) || presence(&field.ty)).unwrap();
    }
    writeln!(out, "*v = next\nreturn nil\n}}\nfunc (v {name}) MarshalJSON() ([]byte, error) {{\nobject := map[string]any{{}}").unwrap();
    if let Some((key, value)) = tag {
        writeln!(out, "object[{key:?}] = {value:?}").unwrap();
    }
    for field in fields {
        let id = ident(&field.json_name);
        if optional(field) {
            writeln!(
                out,
                "if v.{id}.Present {{ object[{:?}] = v.{id} }}",
                field.json_name
            )
            .unwrap();
        } else {
            writeln!(out, "object[{:?}] = v.{id}", field.json_name).unwrap();
        }
    }
    writeln!(out, "return json.Marshal(object)\n}}\n").unwrap();
}

impl Emitter for Go {
    fn owned(&self) -> &'static [&'static str] {
        &["bindings/go/types_gen.go"]
    }

    fn files(&self, model: &Model) -> Vec<(PathBuf, String)> {
        let mut out = String::from(HELPERS);
        for named in &model.types {
            let name = ident(&named.name);
            docs(&mut out, named.doc.as_deref());
            match &named.kind {
                Kind::Struct { fields, .. } => structure(&mut out, &name, fields, None, model),
                Kind::Alias(value) => {
                    writeln!(out, "type {name} = {}\n", ty(value)).unwrap();
                }
                Kind::StringEnum(values) => {
                    writeln!(out, "type {name} string\nconst (").unwrap();
                    for value in values {
                        writeln!(
                            out,
                            "{name}{} {name} = {value:?}",
                            if value.is_empty() {
                                "Empty".into()
                            } else {
                                ident(value)
                            }
                        )
                        .unwrap();
                    }
                    writeln!(out, ")\nfunc (v *{name}) UnmarshalJSON(data []byte) error {{\nvar next string\nif err := json.Unmarshal(data, &next); err != nil {{ return err }}\nswitch next {{").unwrap();
                    for value in values {
                        writeln!(out, "case {value:?}: *v = {name}(next); return nil").unwrap();
                    }
                    writeln!(
                        out,
                        "}}\nreturn fmt.Errorf(\"{name}: unknown value %s\", data)\n}}\n"
                    )
                    .unwrap();
                }
                Kind::Tagged { tag, variants } => {
                    writeln!(out, "type {name}Variant interface {{ is{name}() }}\ntype {name} struct {{ Value {name}Variant }}\nfunc (v {name}) MarshalJSON() ([]byte, error) {{\nif v.Value == nil {{ return nil, fmt.Errorf(\"{name}: missing variant\") }}\nreturn json.Marshal(v.Value)\n}}\nfunc (v *{name}) UnmarshalJSON(data []byte) error {{\nobject, err := dtoObject(data)\nif err != nil {{ return fmt.Errorf(\"{name}: %w\", err) }}\nvar tag string\nif err := dtoRead(object, {tag:?}, &tag, true, false); err != nil {{ return fmt.Errorf(\"{name}: %w\", err) }}").unwrap();
                    for variant in variants {
                        let vn = format!("{name}Variant{}", ident(&variant.name));
                        writeln!(out, "if tag == {:?} {{\nvar next {vn}\nif err := json.Unmarshal(data, &next); err == nil {{ v.Value = next; return nil }}\n}}", variant.tag_value).unwrap();
                    }
                    writeln!(
                        out,
                        "return fmt.Errorf(\"{name}: unknown or invalid {tag} tag %q\", tag)\n}}\n"
                    )
                    .unwrap();
                    for variant in variants {
                        let vn = format!("{name}Variant{}", ident(&variant.name));
                        structure(
                            &mut out,
                            &vn,
                            &variant.fields,
                            Some((tag, &variant.tag_value)),
                            model,
                        );
                        writeln!(out, "func ({vn}) is{name}() {{}}\n").unwrap();
                    }
                }
                Kind::Untagged(branches) => {
                    writeln!(out, "type {name}Variant interface {{ is{name}() }}\ntype {name} struct {{ Value {name}Variant }}\nfunc (v {name}) MarshalJSON() ([]byte, error) {{\nif v.Value == nil {{ return nil, fmt.Errorf(\"{name}: missing variant\") }}\nreturn json.Marshal(v.Value)\n}}\nfunc (v *{name}) UnmarshalJSON(data []byte) error {{").unwrap();
                    for (i, branch) in branches.iter().enumerate() {
                        writeln!(out, "{{\nvar next {}\nif err := dtoDecode(data, &next, {}); err == nil {{ v.Value = {name}Branch{i}{{Value: next}}; return nil }}\n}}", ty(branch), nullable(branch, model)).unwrap();
                    }
                    writeln!(out, "return fmt.Errorf(\"{name}: no matching branch\")\n}}").unwrap();
                    for (i, branch) in branches.iter().enumerate() {
                        writeln!(out, "type {name}Branch{i} struct {{ Value {} }}\nfunc ({name}Branch{i}) is{name}() {{}}\nfunc (v {name}Branch{i}) MarshalJSON() ([]byte, error) {{ return json.Marshal(v.Value) }}\n", ty(branch)).unwrap();
                    }
                }
            }
        }
        // The runner's name table is derived from exactly the same model as the declarations.
        out.push_str("// NewDTO returns an empty generated DTO for contract-vector decoding.\nfunc NewDTO(name string) (any, error) {\nswitch name {\n");
        for named in &model.types {
            writeln!(
                out,
                "case {:?}: return new({}), nil",
                named.name,
                ident(&named.name)
            )
            .unwrap();
        }
        out.push_str("}\nreturn nil, fmt.Errorf(\"unknown DTO %q\", name)\n}\n");
        vec![("bindings/go/types_gen.go".into(), gofmt(&out))]
    }
}

fn gofmt(source: &str) -> String {
    let mut child = Command::new("gofmt")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Go DTO generation requires gofmt on PATH");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(source.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "gofmt: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

const HELPERS: &str = r#"// Code generated by aimux-codegen. DO NOT EDIT.
package aimux

import (
    "bytes"
    "encoding/base64"
    "encoding/json"
    "fmt"
    "strconv"
)

// Field distinguishes an absent key, explicit null, and a typed value.
// Set Present when constructing a value; Null takes precedence over Value.
type Field[T any] struct {
    Value T
    Present bool
    Null bool
}

// Some constructs a present, non-null field.
func Some[T any](value T) Field[T] { return Field[T]{Value: value, Present: true} }

func (v *Field[T]) UnmarshalJSON(data []byte) error {
    var next Field[T]
    next.Present = true
    next.Null = bytes.Equal(bytes.TrimSpace(data), []byte("null"))
    if !next.Null {
        if err := json.Unmarshal(data, &next.Value); err != nil { return err }
    }
    *v = next
    return nil
}
func (v Field[T]) MarshalJSON() ([]byte, error) {
    if !v.Present || v.Null { return []byte("null"), nil }
    return json.Marshal(v.Value)
}

// DecodeBase64 decodes a base64 wire string.
func DecodeBase64(value string) ([]byte, error) { return base64.StdEncoding.DecodeString(value) }

// Uint8Number keeps byte arrays as JSON arrays rather than base64 strings.
type Uint8Number uint8
func (v Uint8Number) MarshalJSON() ([]byte, error) { return []byte(strconv.FormatUint(uint64(v), 10)), nil }

// Uint64String is an unsigned integer encoded as a canonical decimal string.
type Uint64String uint64
func (v Uint64String) MarshalJSON() ([]byte, error) { return json.Marshal(strconv.FormatUint(uint64(v), 10)) }
func (v *Uint64String) UnmarshalJSON(data []byte) error {
    var text string
    if err := json.Unmarshal(data, &text); err != nil { return err }
    next, err := decimalUint64(text)
    if err != nil { return err }
    *v = Uint64String(next)
    return nil
}
func decimalUint64(text string) (uint64, error) {
    if text == "" || (len(text) > 1 && text[0] == '0') { return 0, fmt.Errorf("invalid uint64 %q", text) }
    for _, c := range text { if c < '0' || c > '9' { return 0, fmt.Errorf("invalid uint64 %q", text) } }
    return strconv.ParseUint(text, 10, 64)
}

func dtoObject(data []byte) (map[string]json.RawMessage, error) {
    var object map[string]json.RawMessage
    if err := json.Unmarshal(data, &object); err != nil { return nil, err }
    if object == nil { return nil, fmt.Errorf("expected object") }
    return object, nil
}
func dtoDecode[T any](data []byte, dst *T, nullable bool) error {
    if !nullable && bytes.Equal(bytes.TrimSpace(data), []byte("null")) { return fmt.Errorf("unexpected null") }
    return json.Unmarshal(data, dst)
}
func dtoRead[T any](object map[string]json.RawMessage, key string, dst *T, required, nullable bool) error {
    raw, present := object[key]
    if !present {
        if required { return fmt.Errorf("missing %s", key) }
        return nil
    }
    if err := dtoDecode(raw, dst, nullable); err != nil { return fmt.Errorf("%s: %w", key, err) }
    return nil
}
func dtoTag(object map[string]json.RawMessage, key, want string) error {
    var tag string
    if err := dtoRead(object, key, &tag, true, false); err != nil { return err }
    if tag != want { return fmt.Errorf("expected %s tag %q, got %q", key, want, tag) }
    return nil
}

"#;
