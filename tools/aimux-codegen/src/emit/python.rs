//! Pydantic models and dependency-free error-envelope support.
use super::Emitter;
use crate::model::{Codec, Field, Kind, Model, Result, Ty};
use std::{collections::BTreeMap, fmt::Write, path::PathBuf};

pub struct Python;

impl Emitter for Python {
    fn owned(&self) -> &'static [&'static str] {
        &[
            "bindings/python/python/aimux/_types.py",
            "bindings/python/python/aimux/_error_payloads.py",
        ]
    }

    fn files(&self, model: &Model) -> Result<Vec<(PathBuf, String)>> {
        let mut out = String::from(RUNTIME);
        let mut exports = vec![
            "base64_wire".to_owned(),
            "u64_string".to_owned(),
            "timestamp".to_owned(),
        ];
        let mut classes = Vec::new();
        for named in &model.types {
            let name = &named.name;
            exports.push(name.clone());
            match &named.kind {
                Kind::Struct { fields, closed } => {
                    class(&mut out, name, named.doc.as_deref(), fields, *closed, None);
                    classes.push(name.clone());
                }
                Kind::Tagged { tag, variants } => {
                    let mut groups: BTreeMap<&str, Vec<String>> = BTreeMap::new();
                    for variant in variants {
                        let variant_name = format!("{name}{}", variant.name);
                        class(
                            &mut out,
                            &variant_name,
                            None,
                            &variant.fields,
                            variant.closed,
                            Some((tag, &variant.tag_value)),
                        );
                        groups
                            .entry(&variant.tag_value)
                            .or_default()
                            .push(variant_name.clone());
                        classes.push(variant_name.clone());
                        exports.push(variant_name);
                    }
                    docs(&mut out, named.doc.as_deref(), "");
                    let expression = if groups.values().all(|g| g.len() == 1) {
                        format!(
                            "Annotated[{}, Field(discriminator={})]",
                            union(
                                variants
                                    .iter()
                                    .map(|v| quote(&format!("{name}{}", v.name)))
                                    .collect()
                            ),
                            quote(&ident(tag))
                        )
                    } else {
                        // Flattened unions can repeat the outer tag (e.g. source/url
                        // and source/document). Dispatch that tag, then try its shapes
                        // in their original order.
                        let branches = groups
                            .iter()
                            .map(|(tag, names)| {
                                let inner = ordered_union(names.iter().map(|n| quote(n)).collect());
                                format!("Annotated[{inner}, Tag({})]", quote(tag))
                            })
                            .collect();
                        format!(
                            "Annotated[{}, Field(discriminator=Discriminator(lambda value: _tag(value, {}, {})))]",
                            union(branches),
                            quote(tag),
                            quote(&ident(tag))
                        )
                    };
                    alias(&mut out, name, &expression);
                }
                Kind::StringEnum(values) => {
                    docs(&mut out, named.doc.as_deref(), "");
                    alias(
                        &mut out,
                        name,
                        &format!(
                            "Literal[{}]",
                            values
                                .iter()
                                .map(|v| quote(v))
                                .collect::<Vec<_>>()
                                .join(", ")
                        ),
                    );
                }
                Kind::Untagged(branches) => {
                    docs(&mut out, named.doc.as_deref(), "");
                    alias(
                        &mut out,
                        name,
                        &ordered_union(branches.iter().map(|branch| ty(&branch.ty)).collect()),
                    );
                }
                Kind::Alias(inner) => {
                    docs(&mut out, named.doc.as_deref(), "");
                    alias(&mut out, name, &ty(inner));
                }
            }
        }
        out.push_str("# Resolve recursive references after all declarations exist.\n_dto = sys.modules[__name__]\n");
        for name in classes {
            writeln!(out, "{name}.model_rebuild(_types_namespace=globals())").unwrap();
        }
        out.push_str("\n# Name-to-type table used by the DTO vector runner.\nDTO_TYPES = {\n");
        for named in &model.types {
            writeln!(out, "    {}: {},", quote(&named.name), named.name).unwrap();
        }
        out.push_str("}\n\n__all__ = [\n");
        for name in exports {
            writeln!(out, "    {},", quote(&name)).unwrap();
        }
        out.push_str("]\n");
        Ok(vec![
            (self.owned()[0].into(), out),
            (self.owned()[1].into(), error_support(model)?),
        ])
    }
}

fn quote(value: &str) -> String {
    // JSON string escapes are also valid Python, including control characters.
    serde_json::to_string(value).unwrap()
}

fn ident(value: &str) -> String {
    let mut out = String::new();
    let chars: Vec<_> = value.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        if c.is_ascii_uppercase()
            && i > 0
            && (chars[i - 1].is_ascii_lowercase()
                || chars[i - 1].is_ascii_digit()
                || chars.get(i + 1).is_some_and(char::is_ascii_lowercase))
        {
            out.push('_');
        }
        out.push(if c.is_ascii_alphanumeric() || c == '_' {
            c.to_ascii_lowercase()
        } else {
            '_'
        });
    }
    if out.starts_with('_') || out.as_bytes().first().is_some_and(u8::is_ascii_digit) {
        out.insert_str(0, "field");
    }
    if [
        "False",
        "None",
        "True",
        "and",
        "as",
        "assert",
        "async",
        "await",
        "break",
        "class",
        "continue",
        "def",
        "del",
        "elif",
        "else",
        "except",
        "finally",
        "for",
        "from",
        "global",
        "if",
        "import",
        "in",
        "is",
        "lambda",
        "nonlocal",
        "not",
        "or",
        "pass",
        "raise",
        "return",
        "try",
        "while",
        "with",
        "yield",
        "model_config",
        "model_fields",
        "model_dump",
        "model_validate",
        "schema",
    ]
    .contains(&out.as_str())
    {
        out.push('_');
    }
    out
}

fn docs(out: &mut String, doc: Option<&str>, indent: &str) {
    if let Some(doc) = doc {
        for line in doc.lines() {
            if line.trim().is_empty() {
                writeln!(out, "{indent}#:").unwrap();
            } else {
                writeln!(out, "{indent}#: {}", line.trim_end()).unwrap();
            }
        }
    }
}

fn union(branches: Vec<String>) -> String {
    if branches.len() == 1 {
        branches[0].clone()
    } else {
        format!("Union[{}]", branches.join(", "))
    }
}

fn ordered_union(branches: Vec<String>) -> String {
    if branches.len() == 1 {
        branches[0].clone()
    } else {
        format!(
            "Annotated[{}, Field(union_mode=\"left_to_right\")]",
            union(branches)
        )
    }
}

fn ty(value: &Ty) -> String {
    match value {
        Ty::Ref(name) => quote(&format!("_dto.{name}")),
        Ty::Literal(value) => format!("Literal[{}]", quote(value)),
        Ty::String => "str".into(),
        Ty::Bool => "bool".into(),
        Ty::Int { unsigned, bits } => format!("{}{bits}", if *unsigned { "U" } else { "I" }),
        Ty::Float => "float".into(),
        Ty::Array(inner) => format!("List[{}]", ty(inner)),
        Ty::Map(inner) => format!("Dict[str, {}]", ty(inner)),
        Ty::Json => "Any".into(),
        Ty::Nullable(inner) => format!("Optional[{}]", ty(inner)),
        Ty::Codec(codec, inner) => match codec {
            Codec::Base64 => base64_ty(inner),
            Codec::U64String => "Annotated[str, BeforeValidator(u64_string)]".into(),
            Codec::Timestamp => "Annotated[str, BeforeValidator(timestamp)]".into(),
            Codec::Presence | Codec::Tagged => ty(inner),
        },
    }
}

fn nullable(value: &Ty) -> bool {
    match value {
        Ty::Nullable(_) => true,
        Ty::Codec(_, inner) => nullable(inner),
        _ => false,
    }
}

fn base64_ty(inner: &Ty) -> String {
    match inner {
        Ty::String => "Annotated[str, BeforeValidator(base64_wire)]".into(),
        Ty::Array(item) => format!("List[{}]", base64_ty(item)),
        Ty::Nullable(item) => format!("Optional[{}]", base64_ty(item)),
        _ => ty(inner),
    }
}

fn alias(out: &mut String, name: &str, expression: &str) {
    writeln!(
        out,
        "{name} = TypeAliasType({}, {expression})\n",
        quote(name)
    )
    .unwrap();
}

fn class(
    out: &mut String,
    name: &str,
    doc: Option<&str>,
    fields: &[Field],
    closed: bool,
    tag: Option<(&str, &str)>,
) {
    docs(out, doc, "");
    writeln!(out, "class {name}(_Wire):").unwrap();
    if closed {
        out.push_str("    model_config = ConfigDict(extra=\"forbid\")\n");
    }
    if let Some((key, value)) = tag {
        writeln!(
            out,
            "    {}: Literal[{}] = Field(default={}, alias={})",
            ident(key),
            quote(value),
            quote(value),
            quote(key)
        )
        .unwrap();
    }
    for field in fields {
        docs(out, field.doc.as_deref(), "    ");
        let annotation = if field.required || nullable(&field.ty) {
            ty(&field.ty)
        } else {
            format!("Optional[{}]", ty(&field.ty))
        };
        writeln!(
            out,
            "    {}: {annotation} = Field({}alias={})",
            ident(&field.json_name),
            if field.required { "" } else { "default=None, " },
            quote(&field.json_name)
        )
        .unwrap();
    }
    let omit_null: Vec<_> = fields
        .iter()
        .filter(|field| !field.required && !field.ty.accepts_null())
        .map(|field| quote(&ident(&field.json_name)))
        .collect();
    if !omit_null.is_empty() {
        writeln!(
            out,
            "    _omit_null: ClassVar[frozenset[str]] = frozenset([{}])",
            omit_null.join(", ")
        )
        .unwrap();
    }
    if fields.is_empty() && tag.is_none() && !closed {
        out.push_str("    pass\n");
    }
    out.push_str("\n\n");
}

fn error_support(model: &Model) -> Result<String> {
    let mut out = String::from(
        "# Generated by aimux-codegen. Do not edit.\n\"\"\"Dependency-free views for exception presentation.\"\"\"\nfrom dataclasses import dataclass\nfrom typing import Any\n\n\n@dataclass(frozen=True)\nclass ErrorEnvelope:\n",
    );
    let named = model
        .types
        .iter()
        .find(|t| t.name == "ErrorEnvelope")
        .ok_or("ErrorEnvelope #: missing type")?;
    let Kind::Struct { fields, .. } = &named.kind else {
        return Err("ErrorEnvelope #: expected struct".into());
    };
    for field in fields {
        writeln!(out, "    {}: Any", ident(&field.json_name)).unwrap();
    }
    out.push_str("\n    @classmethod\n    def from_json(cls, value):\n        return cls(\n");
    for field in fields {
        let key = quote(&field.json_name);
        let access = if field.required {
            format!("value[{key}]")
        } else {
            format!("value.get({key})")
        };
        writeln!(out, "            {}={access},", ident(&field.json_name)).unwrap();
    }
    out.push_str("        )\n\n\n");
    let Kind::Tagged { tag, variants } = &model
        .types
        .iter()
        .find(|t| t.name == "AiMuxError")
        .ok_or("AiMuxError #: missing type")?
        .kind
    else {
        return Err("AiMuxError #: expected tagged union".into());
    };
    let required = fields
        .iter()
        .filter(|f| f.required)
        .map(|f| quote(&f.json_name))
        .collect::<Vec<_>>()
        .join(", ");
    writeln!(
        out,
        "_ENVELOPE_KEYS = frozenset([{required}])\n_ERROR_TAG = {}\n",
        quote(tag)
    )
    .unwrap();
    out.push_str("_NESTED_ERROR_FIELDS = {\n");
    for variant in variants {
        let nested: Vec<_> = variant
            .fields
            .iter()
            .filter(|f| contains_error(&f.ty))
            .map(|f| quote(&f.json_name))
            .collect();
        if !nested.is_empty() {
            writeln!(
                out,
                "    {}: frozenset([{}]),",
                quote(&variant.tag_value),
                nested.join(", ")
            )
            .unwrap();
        }
    }
    out.push_str("}\n\n\n");
    out.push_str(ERROR_RUNTIME);
    Ok(out)
}

fn contains_error(ty: &Ty) -> bool {
    match ty {
        Ty::Ref(name) => name == "AiMuxError" || name == "ErrorEnvelope",
        Ty::Array(inner) | Ty::Map(inner) | Ty::Nullable(inner) | Ty::Codec(_, inner) => {
            contains_error(inner)
        }
        _ => false,
    }
}

const RUNTIME: &str = r#"# Generated by aimux-codegen. Do not edit.
"""Wire DTOs from the Rust operation schemas; requires the optional typed extra."""
from __future__ import annotations

import base64
import sys
from datetime import datetime
from typing import Annotated, Any, ClassVar, Dict, List, Literal, Optional, Union

from pydantic import (
    BaseModel, BeforeValidator, ConfigDict, Discriminator, Field, Tag,
    model_serializer,
)
from typing_extensions import TypeAliasType


U8 = Annotated[int, Field(ge=0, le=255)]
U16 = Annotated[int, Field(ge=0, le=65535)]
U32 = Annotated[int, Field(ge=0, le=4294967295)]
U64 = Annotated[int, Field(ge=0, le=18446744073709551615)]
I8 = Annotated[int, Field(ge=-128, le=127)]
I16 = Annotated[int, Field(ge=-32768, le=32767)]
I32 = Annotated[int, Field(ge=-2147483648, le=2147483647)]
I64 = Annotated[int, Field(ge=-9223372036854775808, le=9223372036854775807)]


def base64_wire(value, *, decode=False):
    """Keep wire strings, encode bytes, or decode base64 with decode=True."""
    if isinstance(value, (bytes, bytearray, memoryview)):
        value = base64.b64encode(value).decode("ascii")
    if not isinstance(value, str):
        raise ValueError("expected base64 string or bytes")
    if decode:
        try:
            return base64.b64decode(value, validate=True)
        except (ValueError, UnicodeError) as error:
            raise ValueError("invalid base64 string") from error
    return value


def u64_string(value):
    """Validate a decimal u64 without changing its string representation."""
    if not isinstance(value, str) or not value.isascii() or not value.isdecimal() or int(value) >= (1 << 64):
        raise ValueError("expected decimal u64 string")
    return value


def timestamp(value):
    """Validate an ISO-8601 timestamp while retaining its wire spelling."""
    if not isinstance(value, str):
        raise ValueError("expected ISO-8601 timestamp string")
    try:
        # Python's datetime starts at year 1; Rust's timestamp also accepts 0000.
        checked = "0004" + value[4:] if value.startswith("0000-") else value
        parsed = datetime.fromisoformat(checked.replace("Z", "+00:00"))
    except ValueError as error:
        raise ValueError("invalid ISO-8601 timestamp") from error
    if parsed.tzinfo is None:
        raise ValueError("timestamp requires a timezone")
    return value


def _tag(value, key, attribute):
    return value.get(key) if isinstance(value, dict) else getattr(value, attribute, None)


class _Wire(BaseModel):
    model_config = ConfigDict(populate_by_name=True, strict=True, extra="ignore", allow_inf_nan=False)
    _omit_null: ClassVar[frozenset[str]] = frozenset()

    @model_serializer(mode="wrap")
    def _omit_absent(self, handler, info):
        data = handler(self)
        for name, field in type(self).model_fields.items():
            if (name not in self.model_fields_set and not field.is_required() and field.default is None
                    or name in self._omit_null and getattr(self, name) is None):
                data.pop(field.alias if info.by_alias else name, None)
        return data


"#;

const ERROR_RUNTIME: &str = r#"def payload_fields(payload, payload_exception, envelope_exception):
    """Project nested error values without interpreting provider JSON fields."""
    def project(value):
        if isinstance(value, list):
            return [project(item) for item in value]
        if isinstance(value, dict):
            if _ENVELOPE_KEYS <= value.keys():
                return envelope_exception(value)
            if isinstance(value.get(_ERROR_TAG), str):
                return payload_exception(value)
        return value

    fields = _NESTED_ERROR_FIELDS.get(payload.get(_ERROR_TAG), ())
    return ((key, project(value) if key in fields else value) for key, value in payload.items())
"#;
