//! Go DTOs and their JSON codecs, generated solely from the normalized model.
use super::Emitter;
use crate::model::{Codec, Field, Kind, Model, Ty};
use std::{collections::BTreeMap, fmt::Write, path::PathBuf};

pub struct Go;

fn ident(value: &str) -> String {
    let mut result = value
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|s| !s.is_empty())
        .map(|part| part[..1].to_ascii_uppercase() + &part[1..])
        .collect::<String>();
    if result.starts_with(|c: char| c.is_ascii_digit()) {
        result.insert(0, 'V');
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
            result.replace_range(result.len() - word.len().., initialism);
        } else if result.starts_with(word)
            && result[word.len()..].starts_with(|c: char| c.is_ascii_uppercase())
        {
            result.replace_range(..word.len(), initialism);
        }
    }
    result
}

fn ty(value: &Ty) -> String {
    match value {
        Ty::Ref(name) => ident(name),
        Ty::String | Ty::Literal(_) => "string".into(),
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

fn presence_inner(value: &Ty) -> Option<&Ty> {
    match value {
        Ty::Codec(Codec::Presence, inner) => Some(inner),
        Ty::Nullable(inner) | Ty::Codec(_, inner) => presence_inner(inner),
        _ => None,
    }
}

fn shape<'a>(value: &'a Ty, model: &'a Model) -> &'a Ty {
    match value {
        Ty::Ref(name) => match kind(name, model) {
            Kind::Alias(inner) => shape(inner, model),
            _ => value,
        },
        Ty::Codec(_, inner) => shape(inner, model),
        _ => value,
    }
}

fn field_ty(field: &Field, model: &Model) -> String {
    if let Some(inner) = presence_inner(&field.ty) {
        return format!("Field[{}]", ty(inner));
    }
    if field.required {
        return ty(&field.ty);
    }
    let mut value = &field.ty;
    while let Ty::Nullable(inner) = shape(value, model) {
        value = inner;
    }
    match shape(value, model) {
        Ty::Array(_) | Ty::Map(_) => ty(value),
        _ => format!("*{}", ty(value)),
    }
}

fn docs(out: &mut String, doc: Option<&str>, indent: &str) {
    for line in doc.unwrap_or("").replace("``", "“").lines() {
        let space = if line.is_empty() { "" } else { " " };
        writeln!(out, "{indent}//{space}{line}").unwrap();
    }
}

fn structure(out: &mut String, name: &str, fields: &[Field], model: &Model) {
    if fields.is_empty() {
        writeln!(out, "type {name} struct{{}}\n").unwrap();
        return;
    }
    writeln!(out, "type {name} struct {{").unwrap();
    for group in fields.chunk_by(|_, right| right.doc.is_none()) {
        let rows: Vec<_> = group
            .iter()
            .map(|f| (ident(&f.json_name), field_ty(f, model)))
            .collect();
        let names = rows.iter().map(|r| r.0.len()).max().unwrap();
        let types = rows.iter().map(|r| r.1.len()).max().unwrap();
        docs(out, group[0].doc.as_deref(), "\t");
        for (field, (id, ty)) in group.iter().zip(rows) {
            let omit = if field.required { "" } else { ",omitzero" };
            writeln!(
                out,
                "\t{id:names$} {ty:types$} `json:\"{}{omit}\"`",
                field.json_name
            )
            .unwrap();
        }
    }
    out.push_str("}\n\n");
}

fn kind<'a>(name: &str, model: &'a Model) -> &'a Kind {
    &model.types.iter().find(|t| t.name == name).unwrap().kind
}

// Secondary discriminators select flattened variants that share the outer tag.
fn literals(fields: &[Field]) -> BTreeMap<&str, &str> {
    fields
        .iter()
        .filter_map(|field| {
            if let Ty::Literal(value) = &field.ty {
                Some((field.json_name.as_str(), value.as_str()))
            } else {
                None
            }
        })
        .collect()
}

impl Emitter for Go {
    fn owned(&self) -> &'static [&'static str] {
        &["bindings/go/types_gen.go"]
    }

    fn files(&self, model: &Model) -> Vec<(PathBuf, String)> {
        let mut out = String::from(HELPERS);
        for named in &model.types {
            let name = ident(&named.name);
            docs(&mut out, named.doc.as_deref(), "");
            match &named.kind {
                Kind::Struct { fields, .. } => structure(&mut out, &name, fields, model),
                Kind::Alias(value) => {
                    writeln!(out, "type {name} = {}\n", ty(value)).unwrap();
                }
                Kind::StringEnum(values) => {
                    writeln!(out, "type {name} string\n\nconst (").unwrap();
                    let ids: Vec<_> = values
                        .iter()
                        .map(|v| format!("{name}{}", ident(if v.is_empty() { "Empty" } else { v })))
                        .collect();
                    let width = ids.iter().map(String::len).max().unwrap_or(0);
                    for (value, id) in values.iter().zip(ids) {
                        writeln!(out, "\t{id:width$} {name} = {value:?}").unwrap();
                    }
                    out.push_str(")\n\n");
                }
                Kind::Tagged { tag, variants } => {
                    union(&mut out, &name);
                    writeln!(out, "func (v *{name}) UnmarshalJSON(data []byte) error {{\n\tvar tag struct {{\n\t\t{} string `json:{tag:?}`\n\t}}\n\tif err := json.Unmarshal(data, &tag); err != nil {{\n\t\treturn err\n\t}}\n\tswitch tag.{} {{", ident(tag), ident(tag)).unwrap();
                    let mut groups = BTreeMap::<_, Vec<_>>::new();
                    for variant in variants {
                        groups.entry(&variant.tag_value).or_default().push(variant);
                    }
                    for (value, variants) in groups {
                        writeln!(out, "\tcase {value:?}:").unwrap();
                        let keys: BTreeMap<_, _> =
                            variants.iter().flat_map(|v| literals(&v.fields)).collect();
                        if !keys.is_empty() {
                            out.push_str("\t\tvar subtag struct {\n");
                            let width = keys.keys().map(|k| ident(k).len()).max().unwrap();
                            for key in keys.keys() {
                                let id = ident(key);
                                writeln!(out, "\t\t\t{id:width$} string `json:{key:?}`").unwrap();
                            }
                            out.push_str("\t\t}\n\t\tif err := json.Unmarshal(data, &subtag); err != nil {\n\t\t\treturn err\n\t\t}\n");
                        }
                        for variant in variants {
                            let vn = format!("{name}Variant{}", ident(&variant.name));
                            let condition = literals(&variant.fields)
                                .iter()
                                .map(|(k, v)| format!("subtag.{} == {v:?}", ident(k)))
                                .collect::<Vec<_>>()
                                .join(" && ");
                            let indent = if condition.is_empty() {
                                "\t\t"
                            } else {
                                "\t\t\t"
                            };
                            if !condition.is_empty() {
                                writeln!(out, "\t\tif {condition} {{").unwrap();
                            }
                            writeln!(out, "{indent}var next {vn}\n{indent}if err := json.Unmarshal(data, &next); err != nil {{\n{indent}\treturn err\n{indent}}}\n{indent}v.Value = next\n{indent}return nil").unwrap();
                            if !condition.is_empty() {
                                out.push_str("\t\t}\n");
                            }
                        }
                        if !keys.is_empty() {
                            writeln!(
                                out,
                                "\t\treturn fmt.Errorf(\"{name}: unknown tag %+v\", subtag)"
                            )
                            .unwrap();
                        }
                    }
                    writeln!(
                        out,
                        "\t}}\n\treturn fmt.Errorf(\"{name}: unknown {tag} tag %q\", tag.{})\n}}\n",
                        ident(tag)
                    )
                    .unwrap();
                    for variant in variants {
                        let vn = format!("{name}Variant{}", ident(&variant.name));
                        structure(&mut out, &vn, &variant.fields, model);
                        writeln!(out, "func ({vn}) is{name}() {{}}\n\nfunc (v {vn}) MarshalJSON() ([]byte, error) {{\n\ttype plain {vn}\n\treturn json.Marshal(struct {{\n\t\t{} string `json:{tag:?}`\n\t\tplain\n\t}}{{{}: {:?}, plain: plain(v)}})\n}}\n", ident(tag), ident(tag), variant.tag_value).unwrap();
                    }
                }
                Kind::Untagged(branches) => {
                    union(&mut out, &name);
                    writeln!(
                        out,
                        "func (v *{name}) UnmarshalJSON(data []byte) error {{\n\tvar err error"
                    )
                    .unwrap();
                    for (i, branch) in branches.iter().enumerate() {
                        let mut required = String::new();
                        if let Ty::Ref(reference) = branch
                            && let Kind::Struct { fields, .. } = kind(reference, model)
                        {
                            for field in fields.iter().filter(|f| f.required) {
                                write!(required, ", {:?}", field.json_name).unwrap();
                            }
                        }
                        let nullable = matches!(shape(branch, model), Ty::Nullable(_) | Ty::Json);
                        writeln!(out, "\t{{\n\t\tvar next {}\n\t\tif err = decodeUnionBranch(data, &next, {nullable}{required}); err == nil {{\n\t\t\tv.Value = {name}Branch{i}{{Value: next}}\n\t\t\treturn nil\n\t\t}}\n\t}}", ty(branch)).unwrap();
                    }
                    out.push_str("\treturn err\n}\n\n");
                    for (i, branch) in branches.iter().enumerate() {
                        writeln!(out, "type {name}Branch{i} struct{{ Value {} }}\n\nfunc ({name}Branch{i}) is{name}() {{}}\n\nfunc (v {name}Branch{i}) MarshalJSON() ([]byte, error) {{\n\treturn json.Marshal(v.Value)\n}}\n", ty(branch)).unwrap();
                    }
                }
            }
        }
        out.push_str("// NewDTO returns an empty generated DTO for contract-vector decoding.\nfunc NewDTO(name string) (any, error) {\n\tswitch name {\n");
        for named in &model.types {
            writeln!(
                out,
                "\tcase {:?}:\n\t\treturn new({}), nil",
                named.name,
                ident(&named.name)
            )
            .unwrap();
        }
        out.push_str("\t}\n\treturn nil, fmt.Errorf(\"unknown DTO %q\", name)\n}\n");
        vec![("bindings/go/types_gen.go".into(), out)]
    }
}

fn union(out: &mut String, name: &str) {
    writeln!(out, "type {name}Variant interface{{ is{name}() }}\ntype {name} struct {{\n\tValue {name}Variant\n}}\n\nfunc (v {name}) MarshalJSON() ([]byte, error) {{\n\treturn json.Marshal(v.Value)\n}}\n").unwrap();
}

const HELPERS: &str = r#"// Code generated by aimux-codegen. DO NOT EDIT.
package aimux

import (
	"bytes"
	"encoding/json"
	"fmt"
	"reflect"
	"strconv"
)

// Field distinguishes an absent key, explicit null, and a typed value.
// Set Present when constructing a value; Null takes precedence over Value.
type Field[T any] struct {
	Value   T
	Present bool
	Null    bool
}

func (v Field[T]) IsZero() bool { return !v.Present }

func (v *Field[T]) UnmarshalJSON(data []byte) error {
	v.Present = true
	v.Null = bytes.Equal(bytes.TrimSpace(data), []byte("null"))
	if v.Null {
		return nil
	}
	return json.Unmarshal(data, &v.Value)
}

func (v Field[T]) MarshalJSON() ([]byte, error) {
	if !v.Present || v.Null {
		return []byte("null"), nil
	}
	return json.Marshal(v.Value)
}

// Uint8Number keeps byte arrays as JSON arrays rather than base64 strings.
type Uint8Number uint8

func (v Uint8Number) MarshalJSON() ([]byte, error) { return json.Marshal(uint64(v)) }

// Uint64String is an unsigned integer encoded as a canonical decimal string.
type Uint64String uint64

func (v Uint64String) MarshalJSON() ([]byte, error) {
	return json.Marshal(strconv.FormatUint(uint64(v), 10))
}

func (v *Uint64String) UnmarshalJSON(data []byte) error {
	var text string
	if err := json.Unmarshal(data, &text); err != nil {
		return err
	}
	next, err := strconv.ParseUint(text, 10, 64)
	if err != nil {
		return err
	}
	if strconv.FormatUint(next, 10) != text {
		return &strconv.NumError{Func: "ParseUint", Num: text, Err: strconv.ErrSyntax}
	}
	*v = Uint64String(next)
	return nil
}

// decodeUnionBranch checks required keys only while selecting an untagged branch.
func decodeUnionBranch[T any](data []byte, dst *T, nullable bool, required ...string) error {
	if !nullable && bytes.Equal(bytes.TrimSpace(data), []byte("null")) {
		return &json.UnmarshalTypeError{Value: "null", Type: reflect.TypeFor[T]()}
	}
	if len(required) != 0 {
		var object map[string]json.RawMessage
		if err := json.Unmarshal(data, &object); err != nil {
			return err
		}
		for _, key := range required {
			if _, ok := object[key]; !ok {
				return &json.UnmarshalTypeError{Value: "object", Type: reflect.TypeFor[T]()}
			}
		}
	}
	return json.Unmarshal(data, dst)
}

"#;
