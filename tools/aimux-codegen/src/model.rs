//! Language-independent wire types, normalized from the manifest's schemas.
use serde_json::Value;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};

type Result<T> = std::result::Result<T, String>;

#[derive(Debug, PartialEq)]
pub struct Model {
    pub types: Vec<NamedType>,
}
#[derive(Debug, PartialEq)]
pub struct NamedType {
    pub name: String,
    pub doc: Option<String>,
    pub kind: Kind,
}
#[derive(Debug, PartialEq)]
pub enum Kind {
    /// `closed`: the schema forbids keys it does not list (`additionalProperties: false`).
    Struct {
        fields: Vec<Field>,
        closed: bool,
    },
    StringEnum(Vec<String>),
    Tagged {
        tag: String,
        variants: Vec<Variant>,
    },
    Untagged(Vec<Ty>),
    Alias(Ty),
}
#[derive(Debug, PartialEq)]
pub struct Variant {
    pub tag_value: String,
    pub name: String,
    pub fields: Vec<Field>,
    pub closed: bool,
}
#[derive(Debug, PartialEq)]
pub struct Field {
    pub json_name: String,
    pub ty: Ty,
    pub required: bool,
    pub doc: Option<String>,
}
#[derive(Debug, PartialEq)]
pub enum Ty {
    Ref(String),
    String,
    Bool,
    Int { unsigned: bool, bits: u8 },
    Float,
    Array(Box<Ty>),
    Map(Box<Ty>),
    Json,
    Nullable(Box<Ty>),
    Codec(Codec, Box<Ty>),
}
#[derive(Debug, PartialEq)]
pub enum Codec {
    Base64,
    U64String,
    Presence,
    Timestamp,
    Tagged,
}

fn doc(schema: &Value) -> Option<String> {
    schema["description"].as_str().map(str::to_owned)
}

fn escape(value: &str) -> String {
    value.replace('~', "~0").replace('/', "~1")
}

fn rewrite_refs(value: &mut Value, names: &BTreeMap<String, String>) -> Result<()> {
    match value {
        Value::Object(object) => {
            if let Some(Value::String(reference)) = object.get_mut("$ref") {
                let name = names
                    .get(reference)
                    .ok_or_else(|| format!("unknown reference {reference}"))?;
                *reference = format!("{name}.json");
            }
            for child in object.values_mut() {
                rewrite_refs(child, names)?;
            }
        }
        Value::Array(values) => {
            for child in values {
                rewrite_refs(child, names)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Normalize both contracts, splitting input types when their reachable schemas differ.
///
/// # Errors
/// Returns the containing type and JSON pointer for unsupported schema shapes.
pub fn normalize(type_defs: &Value) -> Result<Model> {
    Ok(normalize_with_schemas(type_defs)?.0)
}

pub(crate) fn contract_names(type_defs: &Value) -> Result<BTreeMap<String, String>> {
    let input = type_defs["input"]
        .as_object()
        .ok_or("typeDefs/input must be an object")?;
    let output = type_defs["output"]
        .as_object()
        .ok_or("typeDefs/output must be an object")?;
    let mut names = BTreeMap::new();
    for (direction, defs) in [("input", input), ("output", output)] {
        for name in defs.keys() {
            if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
                return Err(format!(
                    "{name} #/typeDefs/{direction}: unsafe definition name"
                ));
            }
            names.insert(
                format!("#/typeDefs/{direction}/{}", escape(name)),
                name.clone(),
            );
        }
    }
    // A split propagates through references, including recursive definitions.
    loop {
        let mut changed = false;
        for (name, schema) in input {
            if let Some(other) = output.get(name) {
                let mut schema = schema.clone();
                let mut other = other.clone();
                rewrite_refs(&mut schema, &names)
                    .map_err(|e| format!("{name} #/typeDefs/input: {e}"))?;
                rewrite_refs(&mut other, &names)
                    .map_err(|e| format!("{name} #/typeDefs/output: {e}"))?;
                let key = format!("#/typeDefs/input/{}", escape(name));
                if schema != other && names[&key] == *name {
                    let split = format!("{name}Input");
                    if input.contains_key(&split) || output.contains_key(&split) {
                        return Err(format!(
                            "{name} #/typeDefs/input: split name collides with {split}"
                        ));
                    }
                    names.insert(key, split);
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
    Ok(names)
}

pub(crate) fn normalize_with_schemas(
    type_defs: &Value,
) -> Result<(Model, BTreeMap<String, Value>)> {
    let names = contract_names(type_defs)?;
    let input = type_defs["input"].as_object().unwrap();
    let output = type_defs["output"].as_object().unwrap();
    let mut schemas = BTreeMap::new();
    for (direction, defs) in [("input", input), ("output", output)] {
        for (name, schema) in defs {
            let name = names[&format!("#/typeDefs/{direction}/{}", escape(name))].clone();
            let mut schema = schema.clone();
            rewrite_refs(&mut schema, &names).map_err(|e| format!("{name} #: {e}"))?;
            schemas.insert(name, schema);
        }
    }
    let mut normalizer = Normalizer {
        schemas,
        types: BTreeMap::new(),
        pending: BTreeSet::new(),
    };
    for name in normalizer.schemas.keys().cloned().collect::<Vec<_>>() {
        normalizer.named(&name)?;
    }
    Ok((
        Model {
            types: normalizer.types.into_values().collect(),
        },
        normalizer.schemas,
    ))
}

impl Model {
    /// Op roots such as `()`, `String` or bytes are named only so descriptors
    /// can reference them; languages use their built-in types instead.
    pub fn drop_builtin_aliases(&mut self) {
        let mut referenced = BTreeSet::new();
        for named in &self.types {
            named.kind.refs(&mut referenced);
        }
        let referenced: BTreeSet<String> = referenced.into_iter().map(str::to_owned).collect();
        self.types.retain(|t| {
            !matches!(&t.kind, Kind::Alias(ty) if ty.is_builtin()) || referenced.contains(&t.name)
        });
    }
}

impl Kind {
    fn refs<'a>(&'a self, out: &mut BTreeSet<&'a str>) {
        let fields = |fields: &'a [Field], out: &mut BTreeSet<&'a str>| {
            fields.iter().for_each(|field| field.ty.refs(out));
        };
        match self {
            Kind::Struct { fields: items, .. } => fields(items, out),
            Kind::Tagged { variants, .. } => variants.iter().for_each(|v| fields(&v.fields, out)),
            Kind::Untagged(branches) => branches.iter().for_each(|ty| ty.refs(out)),
            Kind::Alias(ty) => ty.refs(out),
            Kind::StringEnum(_) => {}
        }
    }
}

impl Ty {
    fn refs<'a>(&'a self, out: &mut BTreeSet<&'a str>) {
        match self {
            Ty::Ref(name) => {
                out.insert(name);
            }
            Ty::Array(inner) | Ty::Map(inner) | Ty::Nullable(inner) | Ty::Codec(_, inner) => {
                inner.refs(out);
            }
            _ => {}
        }
    }

    fn is_builtin(&self) -> bool {
        match self {
            Ty::Ref(_) | Ty::Codec(..) => false,
            Ty::Array(inner) | Ty::Map(inner) | Ty::Nullable(inner) => inner.is_builtin(),
            _ => true,
        }
    }
}

struct Normalizer {
    schemas: BTreeMap<String, Value>,
    types: BTreeMap<String, NamedType>,
    pending: BTreeSet<String>,
}

impl Normalizer {
    fn fail(name: &str, pointer: &str, message: &str) -> String {
        format!("{name} {pointer}: {message}")
    }

    fn named(&mut self, name: &str) -> Result<()> {
        if self.types.contains_key(name) || !self.pending.insert(name.to_owned()) {
            return Ok(());
        }
        let schema = self.schemas[name].clone();
        let kind = self.kind(&schema, name, "#")?;
        self.types.insert(
            name.to_owned(),
            NamedType {
                name: name.to_owned(),
                doc: doc(&schema),
                kind,
            },
        );
        self.pending.remove(name);
        Ok(())
    }

    fn lift(&mut self, schema: &Value, name: &str, pointer: &str) -> Result<Ty> {
        let suffix: String = pointer
            .split('/')
            .skip(1)
            .filter(|part| !["properties", "oneOf", "anyOf"].contains(part))
            .map(pascal)
            .collect();
        let base = format!("{name}{suffix}");
        let mut candidate = base.clone();
        let mut index = 2;
        while let Some(existing) = self.schemas.get(&candidate) {
            if existing == schema {
                return Ok(Ty::Ref(candidate));
            }
            candidate = format!("{base}{index}");
            index += 1;
        }
        self.schemas.insert(candidate.clone(), schema.clone());
        self.named(&candidate)?;
        Ok(Ty::Ref(candidate))
    }

    fn fields(
        &mut self,
        schema: &Value,
        name: &str,
        pointer: &str,
        tag: Option<&str>,
    ) -> Result<Vec<Field>> {
        let mut fields = Vec::new();
        if let Some(properties) = schema["properties"].as_object() {
            let mut properties: Vec<_> = properties.iter().collect();
            properties.sort_by_key(|(key, _)| *key);
            for (key, value) in properties {
                if tag == Some(key) {
                    continue;
                }
                let path = format!("{pointer}/properties/{}", escape(key));
                fields.push(Field {
                    json_name: key.clone(),
                    ty: self.ty(value, name, &path)?,
                    required: schema["required"]
                        .as_array()
                        .is_some_and(|r| r.iter().any(|v| v == key)),
                    doc: doc(value),
                });
            }
        }
        Ok(fields)
    }

    fn branches(&self, schema: &Value, name: &str, pointer: &str) -> Result<Vec<Value>> {
        let mut schema = schema.clone();
        if let Some(reference) = schema["$ref"].as_str() {
            let reference = reference
                .strip_suffix(".json")
                .ok_or_else(|| Self::fail(name, pointer, "invalid variant reference"))?;
            let target = self
                .schemas
                .get(reference)
                .ok_or_else(|| Self::fail(name, pointer, "unresolved variant reference"))?;
            if !target.is_object() || target.get("$ref").is_some() {
                return Err(Self::fail(
                    name,
                    pointer,
                    "expected object or union variant reference",
                ));
            }
            let mut overlay = schema.clone();
            overlay.as_object_mut().unwrap().remove("$ref");
            schema = target.clone();
            if let Some(object) = schema.as_object_mut() {
                object.remove("description");
            }
            let properties = overlay.as_object_mut().unwrap().remove("properties");
            let required = overlay.as_object_mut().unwrap().remove("required");
            if let Some(properties) = properties {
                for (key, value) in properties.as_object().unwrap() {
                    schema["properties"][key] = value.clone();
                }
            }
            if let Some(required) = required {
                let mut keys = schema["required"].as_array().cloned().unwrap_or_default();
                for key in required.as_array().unwrap() {
                    if !keys.contains(key) {
                        keys.push(key.clone());
                    }
                }
                schema["required"] = json!(keys);
            }
            for (key, value) in overlay.as_object().unwrap() {
                schema[key] = value.clone();
            }
        }
        if let Some(variants) = schema["oneOf"].as_array() {
            let mut overlay = schema.clone();
            overlay.as_object_mut().unwrap().remove("oneOf");
            let mut result = Vec::new();
            for (i, variant) in variants.iter().enumerate() {
                let path = format!("{pointer}/oneOf/{i}");
                for mut branch in self.branches(variant, name, &path)? {
                    if let Some(properties) = overlay["properties"].as_object() {
                        for (key, value) in properties {
                            branch["properties"][key] = value.clone();
                        }
                    }
                    let mut required = branch["required"].as_array().cloned().unwrap_or_default();
                    if let Some(keys) = overlay["required"].as_array() {
                        for key in keys {
                            if !required.contains(key) {
                                required.push(key.clone());
                            }
                        }
                    }
                    branch["required"] = json!(required);
                    result.push(branch);
                }
            }
            Ok(result)
        } else {
            Ok(vec![schema])
        }
    }

    fn kind(&mut self, schema: &Value, name: &str, pointer: &str) -> Result<Kind> {
        self.keywords(schema, name, pointer)?;
        if schema.get("$ref").is_some() && schema.get("properties").is_some() {
            let branches = self.branches(schema, name, pointer)?;
            return if branches.len() == 1 {
                self.kind(&branches[0], name, pointer)
            } else {
                self.kind(&json!({"oneOf":branches}), name, pointer)
            };
        }
        if let Some(parts) = schema["allOf"].as_array() {
            if parts.is_empty() {
                return Err(Self::fail(name, pointer, "empty intersection"));
            }
            // Distribute an object tag across an internally tagged union.
            if parts.len() == 2 && parts[0]["oneOf"].is_array() && parts[1]["type"] == "object" {
                let branches = parts[0]["oneOf"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|branch| json!({"allOf":[branch, parts[1]]}))
                    .map(|intersection| self.merge_objects(&intersection, name, pointer))
                    .collect::<Result<Vec<_>>>()?;
                return self.kind(&json!({"oneOf":branches}), name, pointer);
            }
            if parts.iter().all(|part| part["type"] == "object") {
                let merged = self.merge_objects(schema, name, pointer)?;
                return self.kind(&merged, name, pointer);
            }
            if !parts
                .iter()
                .all(|part| part.get("if").is_some() && part.get("then").is_some())
            {
                return Err(Self::fail(
                    name,
                    &format!("{pointer}/allOf"),
                    "unsupported intersection",
                ));
            }
        }
        if let Some(values) = schema["enum"].as_array() {
            let strings = values
                .iter()
                .map(|v| {
                    v.as_str()
                        .map(str::to_owned)
                        .ok_or_else(|| Self::fail(name, pointer, "non-string enum"))
                })
                .collect::<Result<_>>()?;
            return Ok(Kind::StringEnum(strings));
        }
        if let Some(value) = schema["const"].as_str() {
            return Ok(Kind::StringEnum(vec![value.to_owned()]));
        }
        if let Some(branches) = schema["oneOf"].as_array() {
            if branches.is_empty() {
                return Err(Self::fail(name, pointer, "empty union"));
            }
            if branches.iter().all(|branch| {
                branch["const"].is_string()
                    || branch["enum"]
                        .as_array()
                        .is_some_and(|items| items.iter().all(Value::is_string))
            }) {
                let values = branches
                    .iter()
                    .flat_map(|branch| {
                        branch["enum"]
                            .as_array()
                            .cloned()
                            .unwrap_or_else(|| vec![branch["const"].clone()])
                    })
                    .map(|value| value.as_str().unwrap().to_owned())
                    .collect();
                return Ok(Kind::StringEnum(values));
            }
            let branches = branches
                .iter()
                .enumerate()
                .map(|(i, branch)| self.branches(branch, name, &format!("{pointer}/oneOf/{i}")))
                .collect::<Result<Vec<_>>>()?
                .into_iter()
                .flatten()
                .collect::<Vec<_>>();
            let tag = branches
                .first()
                .and_then(|branch| branch["properties"].as_object())
                .and_then(|props| {
                    props
                        .iter()
                        .find(|(key, value)| {
                            value["const"].is_string()
                                && branches
                                    .iter()
                                    .all(|b| b["properties"][key.as_str()]["const"].is_string())
                        })
                        .map(|(key, _)| key.clone())
                });
            if let Some(tag) = tag {
                let mut variants = Vec::new();
                for (index, branch) in branches.iter().enumerate() {
                    let path = format!("{pointer}/oneOf/{index}");
                    self.keywords(branch, name, &path)?;
                    let value = branch["properties"][&tag]["const"].as_str().unwrap();
                    variants.push(Variant {
                        tag_value: value.to_owned(),
                        name: if branches
                            .iter()
                            .filter(|b| b["properties"][&tag]["const"] == value)
                            .count()
                            > 1
                        {
                            format!("{}{}", pascal(value), index + 1)
                        } else {
                            pascal(value)
                        },
                        fields: self.fields(branch, name, &path, Some(&tag))?,
                        closed: branch["additionalProperties"] == false,
                    });
                }
                return Ok(Kind::Tagged { tag, variants });
            }
            return Err(Self::fail(
                name,
                &format!("{pointer}/oneOf"),
                "expected internally tagged variants",
            ));
        }
        if schema["x-aimux-codec"] == "ordered-untagged" {
            let branches = schema["anyOf"]
                .as_array()
                .ok_or_else(|| Self::fail(name, pointer, "ordered-untagged requires anyOf"))?;
            if branches.is_empty() {
                return Err(Self::fail(name, pointer, "empty union"));
            }
            return Ok(Kind::Untagged(
                branches
                    .iter()
                    .enumerate()
                    .map(|(i, branch)| self.ty(branch, name, &format!("{pointer}/anyOf/{i}")))
                    .collect::<Result<_>>()?,
            ));
        }
        if schema["type"] == "object"
            && schema
                .get("additionalProperties")
                .is_none_or(|v| v == false)
        {
            return Ok(Kind::Struct {
                fields: self.fields(schema, name, pointer, None)?,
                closed: schema["additionalProperties"] == false,
            });
        }
        Ok(Kind::Alias(self.ty(schema, name, pointer)?))
    }

    fn merge_objects(&self, schema: &Value, name: &str, pointer: &str) -> Result<Value> {
        let parts = schema["allOf"].as_array().unwrap();
        let mut merged = json!({"type":"object", "properties":{}, "required":[]});
        for (i, part) in parts.iter().enumerate() {
            self.keywords(part, name, &format!("{pointer}/allOf/{i}"))?;
            if part.get("additionalProperties").is_some() {
                return Err(Self::fail(name, pointer, "closed object intersection"));
            }
            if let Some(properties) = part["properties"].as_object() {
                for (key, value) in properties {
                    if merged["properties"]
                        .get(key)
                        .is_some_and(|old| old != value)
                    {
                        return Err(Self::fail(name, pointer, "conflicting intersection field"));
                    }
                    merged["properties"][key] = value.clone();
                }
            }
            if let Some(required) = part["required"].as_array() {
                for key in required {
                    if !merged["required"].as_array().unwrap().contains(key) {
                        merged["required"].as_array_mut().unwrap().push(key.clone());
                    }
                }
            }
        }
        Ok(merged)
    }

    fn keywords(&self, schema: &Value, name: &str, pointer: &str) -> Result<()> {
        if schema == true {
            return Ok(());
        }
        let object = schema
            .as_object()
            .ok_or_else(|| Self::fail(name, pointer, "expected schema object or true"))?;
        let supported = [
            "allOf",
            "$ref",
            "type",
            "format",
            "properties",
            "required",
            "additionalProperties",
            "items",
            "enum",
            "const",
            "oneOf",
            "anyOf",
            "description",
            "title",
            "default",
            "examples",
            "deprecated",
            "readOnly",
            "writeOnly",
            "minimum",
            "maximum",
            "exclusiveMinimum",
            "exclusiveMaximum",
            "multipleOf",
            "minLength",
            "maxLength",
            "pattern",
            "not",
            "minItems",
            "maxItems",
            "uniqueItems",
            "x-aimux-codec",
        ];
        if let Some(codec) = object.get("x-aimux-codec")
            && !matches!(
                codec.as_str(),
                Some(
                    "base64"
                        | "u64-string"
                        | "presence"
                        | "timestamp"
                        | "tagged"
                        | "ordered-untagged"
                )
            )
        {
            return Err(Self::fail(
                name,
                &format!("{pointer}/x-aimux-codec"),
                "unknown codec",
            ));
        }
        for key in object.keys() {
            if !supported.contains(&key.as_str()) {
                return Err(Self::fail(
                    name,
                    &format!("{pointer}/{}", escape(key)),
                    "unsupported keyword",
                ));
            }
        }
        Ok(())
    }

    fn ty(&mut self, schema: &Value, name: &str, pointer: &str) -> Result<Ty> {
        if schema == true {
            return Ok(Ty::Json);
        }
        let object = schema
            .as_object()
            .ok_or_else(|| Self::fail(name, pointer, "expected schema object or true"))?;
        if !object.is_empty()
            && object
                .keys()
                .all(|k| ["description", "title", "default", "examples"].contains(&k.as_str()))
        {
            return Ok(Ty::Json);
        }
        self.keywords(schema, name, pointer)?;
        if schema["not"] == json!({"type":"null"})
            && object
                .keys()
                .all(|key| ["not", "description", "title"].contains(&key.as_str()))
        {
            return Ok(Ty::Json);
        }
        if let Some(codec) = object.get("x-aimux-codec")
            && codec != "ordered-untagged"
        {
            let codec = match codec.as_str() {
                Some("base64") => Codec::Base64,
                Some("u64-string") => Codec::U64String,
                Some("presence") => Codec::Presence,
                Some("timestamp") => Codec::Timestamp,
                Some("tagged") => Codec::Tagged,
                _ => {
                    return Err(Self::fail(
                        name,
                        &format!("{pointer}/x-aimux-codec"),
                        "unknown codec",
                    ));
                }
            };
            let mut inner = schema.clone();
            inner.as_object_mut().unwrap().remove("x-aimux-codec");
            // An extension on an unconstrained Value schema is still explicitly JSON.
            if inner
                .as_object()
                .unwrap()
                .keys()
                .all(|key| ["description", "default", "title"].contains(&key.as_str()))
            {
                return Ok(Ty::Codec(codec, Box::new(Ty::Json)));
            }
            return Ok(Ty::Codec(codec, Box::new(self.ty(&inner, name, pointer)?)));
        }
        if schema.get("$ref").is_some() && schema.get("properties").is_some() {
            return self.lift(schema, name, pointer);
        }
        if let Some(reference) = schema["$ref"].as_str() {
            let reference = reference
                .strip_suffix(".json")
                .ok_or_else(|| Self::fail(name, pointer, "invalid reference"))?;
            if !self.schemas.contains_key(reference) {
                return Err(Self::fail(name, pointer, "unresolved reference"));
            }
            return Ok(Ty::Ref(reference.to_owned()));
        }
        if let Some(types) = schema["type"].as_array() {
            if types.len() == 2 && types.iter().any(|v| v == "null") {
                let mut inner = schema.clone();
                inner["type"] = types.iter().find(|v| *v != "null").unwrap().clone();
                return Ok(Ty::Nullable(Box::new(self.ty(&inner, name, pointer)?)));
            }
            return Err(Self::fail(
                name,
                &format!("{pointer}/type"),
                "unsupported type union",
            ));
        }
        if let Some(branches) = schema["anyOf"].as_array()
            && branches.len() == 2
            && branches.iter().any(|v| v["type"] == "null")
        {
            return Ok(Ty::Nullable(Box::new(self.ty(
                branches.iter().find(|v| v["type"] != "null").unwrap(),
                name,
                &format!("{pointer}/anyOf"),
            )?)));
        }
        if schema.get("allOf").is_some()
            || schema.get("oneOf").is_some()
            || schema.get("anyOf").is_some()
            || schema.get("enum").is_some()
            || schema.get("const").is_some()
        {
            if pointer == "#" {
                return Err(Self::fail(name, pointer, "unsupported union or literal"));
            }
            return self.lift(schema, name, pointer);
        }
        match schema["type"].as_str() {
            Some("string") => Ok(Ty::String),
            Some("boolean") => Ok(Ty::Bool),
            Some("integer") => {
                let format = schema["format"].as_str().unwrap_or("int64");
                let unsigned = format.starts_with('u') || schema["minimum"].as_u64().is_some();
                let bits = match format
                    .trim_start_matches(['u', 'i'])
                    .trim_start_matches("nt")
                {
                    "8" => 8,
                    "16" => 16,
                    "32" => 32,
                    "64" | "size" | "" => 64,
                    _ => {
                        return Err(Self::fail(
                            name,
                            &format!("{pointer}/format"),
                            "unknown integer format",
                        ));
                    }
                };
                Ok(Ty::Int { unsigned, bits })
            }
            Some("number") => Ok(Ty::Float),
            Some("array") => Ok(Ty::Array(Box::new(
                self.ty(
                    schema
                        .get("items")
                        .ok_or_else(|| Self::fail(name, pointer, "missing array items"))?,
                    name,
                    &format!("{pointer}/items"),
                )?,
            ))),
            Some("object")
                if schema
                    .get("additionalProperties")
                    .is_some_and(|v| v != false) =>
            {
                if schema["properties"]
                    .as_object()
                    .is_some_and(|p| !p.is_empty())
                {
                    return Err(Self::fail(name, pointer, "mixed map and struct"));
                }
                Ok(Ty::Map(Box::new(self.ty(
                    &schema["additionalProperties"],
                    name,
                    &format!("{pointer}/additionalProperties"),
                )?)))
            }
            Some("object") if pointer != "#" => self.lift(schema, name, pointer),
            // Unit has only null inhabitants; Nullable<Json> is the IR's null-capable primitive.
            Some("null") => Ok(Ty::Nullable(Box::new(Ty::Json))),
            _ => Err(Self::fail(name, pointer, "unrepresentable schema")),
        }
    }
}

fn pascal(value: &str) -> String {
    value
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|s| !s.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            chars.next().unwrap().to_ascii_uppercase().to_string() + chars.as_str()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn model(schema: Value) -> Model {
        normalize(&json!({"input": {}, "output": {"Example": schema}})).unwrap()
    }
    #[test]
    fn all_kinds_and_codecs() {
        assert!(matches!(
            model(
                json!({"type":"object","properties":{"text":{"type":"string"}},"required":["text"]})
            )
            .types[0]
                .kind,
            Kind::Struct { closed: false, .. }
        ));
        assert!(matches!(
            model(json!({"type":"object","properties":{},"additionalProperties":false})).types[0]
                .kind,
            Kind::Struct { closed: true, .. }
        ));
        assert!(matches!(
            model(json!({"type":"string","enum":["a","b"]})).types[0].kind,
            Kind::StringEnum(_)
        ));
        assert!(matches!(model(json!({"oneOf":[{"type":"object","properties":{"type":{"const":"first"},"x":{"type":"boolean"}},"required":["type","x"]}]})).types[0].kind, Kind::Tagged { .. }));
        assert!(matches!(model(json!({"anyOf":[{"type":"string"},{"type":"boolean"}],"x-aimux-codec":"ordered-untagged"})).types[0].kind, Kind::Untagged(_)));
        assert_eq!(
            model(json!({"type":"array","items":{"type":"integer","format":"uint32"}})).types[0]
                .kind,
            Kind::Alias(Ty::Array(Box::new(Ty::Int {
                unsigned: true,
                bits: 32
            })))
        );
        for (wire, codec) in [
            ("base64", Codec::Base64),
            ("u64-string", Codec::U64String),
            ("presence", Codec::Presence),
            ("timestamp", Codec::Timestamp),
            ("tagged", Codec::Tagged),
        ] {
            assert_eq!(
                model(json!({"type":"string","x-aimux-codec":wire})).types[0].kind,
                Kind::Alias(Ty::Codec(codec, Box::new(Ty::String)))
            );
        }
    }
    #[test]
    fn primitives_fields_and_nullable_references() {
        for (schema, ty) in [
            (json!(true), Ty::Json),
            (json!({"type":"boolean"}), Ty::Bool),
            (json!({"type":"number"}), Ty::Float),
            (
                json!({"type":"integer","format":"int16"}),
                Ty::Int {
                    unsigned: false,
                    bits: 16,
                },
            ),
            (
                json!({"type":"integer","format":"uint"}),
                Ty::Int {
                    unsigned: true,
                    bits: 64,
                },
            ),
            (
                json!({"type":["string","null"]}),
                Ty::Nullable(Box::new(Ty::String)),
            ),
        ] {
            assert_eq!(model(schema).types[0].kind, Kind::Alias(ty));
        }
        let m = model(json!({"description":"Root", "type":"object", "properties":{
            "z":{"type":"string"}, "a":{"type":"boolean","description":"A field"}
        }, "required":["a"]}));
        assert_eq!(m.types[0].doc.as_deref(), Some("Root"));
        assert_eq!(
            m.types[0].kind,
            Kind::Struct {
                closed: false,
                fields: vec![
                    Field {
                        json_name: "a".into(),
                        ty: Ty::Bool,
                        required: true,
                        doc: Some("A field".into())
                    },
                    Field {
                        json_name: "z".into(),
                        ty: Ty::String,
                        required: false,
                        doc: None
                    },
                ]
            }
        );
    }

    #[test]
    fn tagged_references_keep_payload_fields_and_nested_discriminators() {
        let model = normalize(&json!({"input":{}, "output":{
            "Payload":{"type":"object","properties":{"text":{"type":"string"}},"required":["text"]},
            "Event":{"oneOf":[{"$ref":"#/typeDefs/output/Payload","type":"object","properties":{"type":{"const":"text"}},"required":["type"]}]},
            "Nested":{"oneOf":[{"type":"object","properties":{"type":{"const":"source"}},"required":["type"],"oneOf":[
                {"type":"object","properties":{"sourceType":{"const":"url"},"url":{"type":"string"}},"required":["sourceType","url"]},
                {"type":"object","properties":{"sourceType":{"const":"document"},"title":{"type":"string"}},"required":["sourceType","title"]}
            ]}, {"type":"object","properties":{"type":{"const":"text"},"text":{"type":"string"}},"required":["type","text"]}]}
        }})).unwrap();
        let Kind::Tagged { variants, .. } =
            &model.types.iter().find(|t| t.name == "Event").unwrap().kind
        else {
            panic!()
        };
        assert_eq!(
            variants[0].fields,
            vec![Field {
                json_name: "text".into(),
                ty: Ty::String,
                required: true,
                doc: None
            }]
        );
        let Kind::Tagged { variants, .. } = &model
            .types
            .iter()
            .find(|t| t.name == "Nested")
            .unwrap()
            .kind
        else {
            panic!()
        };
        assert_eq!(variants.len(), 3);
        assert_eq!(variants[0].tag_value, "source");
        assert_ne!(variants[0].name, variants[1].name);
        assert!(
            variants[0]
                .fields
                .iter()
                .any(|f| f.json_name == "url" && f.required)
        );
        assert!(
            variants[1]
                .fields
                .iter()
                .any(|f| f.json_name == "title" && f.required)
        );
    }

    #[test]
    fn split_propagates_and_equal_contracts_merge() {
        let defs = json!({"input":{"A":{"$ref":"#/typeDefs/input/B"},"B":{"type":"string"}},"output":{"A":{"$ref":"#/typeDefs/output/B"},"B":{"type":"boolean"}}});
        let m = normalize(&defs).unwrap();
        assert_eq!(
            m.types.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
            ["A", "AInput", "B", "BInput"]
        );
        assert_eq!(m.types[1].kind, Kind::Alias(Ty::Ref("BInput".into())));
        assert_eq!(
            normalize(&json!({"input":{"A":true},"output":{"A":true}}))
                .unwrap()
                .types
                .len(),
            1
        );
    }
    #[test]
    fn unsupported_shapes_name_type_and_pointer() {
        for schema in [
            json!({}),
            json!({"type":"array"}),
            json!({"type":"string","x-aimux-codec":"future"}),
            json!({"allOf":[]}),
        ] {
            let err = normalize(&json!({"input":{},"output":{"Broken":schema}})).unwrap_err();
            assert!(err.contains("Broken #"), "{err}");
        }
        assert_eq!(
            model(json!({"anyOf":[{"type":"string"},{"type":"null"}]})).types[0].kind,
            Kind::Alias(Ty::Nullable(Box::new(Ty::String)))
        );
        assert_eq!(
            model(json!({"type":"object","additionalProperties":true})).types[0].kind,
            Kind::Alias(Ty::Map(Box::new(Ty::Json)))
        );
    }
}
