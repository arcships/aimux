//! Language-independent wire types, normalized from the manifest's schemas.
use serde_json::Value;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};

pub(crate) type Result<T> = std::result::Result<T, String>;

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
    Untagged(Vec<Branch>),
    Alias(Ty),
}
#[derive(Debug, PartialEq)]
pub struct Branch {
    pub name: String,
    pub ty: Ty,
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
    Literal(String),
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

fn without_descriptions(mut schema: Value) -> Value {
    if let Value::Object(object) = &mut schema {
        object.remove("description");
        for (key, value) in object {
            match key.as_str() {
                "properties" | "$defs" => {
                    if let Some(properties) = value.as_object_mut() {
                        for property in properties.values_mut() {
                            *property = without_descriptions(property.take());
                        }
                    }
                }
                "oneOf" | "anyOf" | "allOf" => {
                    if let Some(branches) = value.as_array_mut() {
                        for branch in branches {
                            *branch = without_descriptions(branch.take());
                        }
                    }
                }
                "items" | "additionalProperties" | "not" | "if" | "then" | "else" => {
                    *value = without_descriptions(value.take());
                }
                _ => {}
            }
        }
    }
    schema
}

pub(crate) fn escape(value: &str) -> String {
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
    // Schemars numbers distinct Rust identities even when their wire shapes agree.
    // Merge only numbered siblings, and repeat because their references may merge too.
    loop {
        let mut changed = false;
        for (direction, defs) in [("input", input), ("output", output)] {
            for (name, schema) in defs {
                let base = name.trim_end_matches(|c: char| c.is_ascii_digit());
                if base == name || !defs.contains_key(base) {
                    continue;
                }
                let key = format!("#/typeDefs/{direction}/{}", escape(name));
                let target = &names[&format!("#/typeDefs/{direction}/{}", escape(base))];
                if names[&key] == *target {
                    continue;
                }
                let mut schema = schema.clone();
                let mut other = defs[base].clone();
                rewrite_refs(&mut schema, &names)?;
                rewrite_refs(&mut other, &names)?;
                if without_descriptions(schema) == without_descriptions(other) {
                    names.insert(key, target.clone());
                    changed = true;
                }
            }
        }
        if !changed {
            break;
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
                if without_descriptions(schema) != without_descriptions(other)
                    && names[&key] == *name
                {
                    let split = format!("{name}Input");
                    if input.contains_key(&split) || output.contains_key(&split) {
                        return Err(format!(
                            "{name} #/typeDefs/input: split name collides with {split}"
                        ));
                    }
                    for (reference, target) in &mut names {
                        if reference.starts_with("#/typeDefs/input/") && target == name {
                            *target = split.clone();
                        }
                    }
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
    let mut schemas = schema_definitions(type_defs, &names)?;
    schemas.retain(|name, _| !name.starts_with('_'));
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

fn resolve_aliases(
    schema: &mut Value,
    aliases: &BTreeMap<String, Value>,
    pending: &mut BTreeSet<String>,
    name: &str,
    pointer: &str,
) -> Result<()> {
    if let Some(reference) = schema["$ref"].as_str().map(str::to_owned)
        && let Some(target) = aliases.get(&reference)
    {
        if !pending.insert(reference.clone()) {
            return Err(format!("recursive path alias {reference}"));
        }
        let mut target = target.clone();
        resolve_aliases(&mut target, aliases, pending, name, pointer)?;
        pending.remove(&reference);
        let overlay = schema
            .as_object_mut()
            .ok_or_else(|| Normalizer::fail(name, pointer, "expected alias object"))?;
        overlay.remove("$ref");
        for (key, value) in std::mem::take(overlay) {
            target[&key] = value;
        }
        *schema = target;
    }
    match schema {
        Value::Object(object) => {
            for (key, value) in object {
                resolve_aliases(
                    value,
                    aliases,
                    pending,
                    name,
                    &format!("{pointer}/{}", escape(key)),
                )?;
            }
        }
        Value::Array(values) => {
            for (i, value) in values.iter_mut().enumerate() {
                resolve_aliases(value, aliases, pending, name, &format!("{pointer}/{i}"))?;
            }
        }
        _ => {}
    }
    Ok(())
}

pub(crate) fn schema_definitions(
    type_defs: &Value,
    names: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, Value>> {
    let input = type_defs["input"]
        .as_object()
        .ok_or("typeDefs #/input: expected object")?;
    let output = type_defs["output"]
        .as_object()
        .ok_or("typeDefs #/output: expected object")?;
    let mut schemas = BTreeMap::new();
    for (direction, defs) in [("input", input), ("output", output)] {
        for (name, schema) in defs {
            let name = names[&format!("#/typeDefs/{direction}/{}", escape(name))].clone();
            let mut schema = schema.clone();
            rewrite_refs(&mut schema, names).map_err(|e| format!("{name} #: {e}"))?;
            schemas.entry(name).or_insert(schema);
        }
    }
    let aliases: BTreeMap<_, _> = schemas
        .iter()
        .filter(|(name, _)| name.starts_with('_'))
        .map(|(name, schema)| (format!("{name}.json"), schema.clone()))
        .collect();
    for (name, schema) in &mut schemas {
        resolve_aliases(schema, &aliases, &mut BTreeSet::new(), name, "#")?;
    }
    Ok(schemas)
}

pub(crate) fn root_schema(definitions: &BTreeMap<String, Value>, name: &str) -> Result<Value> {
    if !name.starts_with('_') {
        return Ok(json!({"$ref": format!("../types/{name}.json")}));
    }
    // Primitive and collection operation roots are inline, rather than public aliases.
    let mut schema = definitions[name].clone();
    let relative = definitions
        .keys()
        .map(|name| (format!("{name}.json"), format!("../types/{name}")))
        .collect();
    rewrite_refs(&mut schema, &relative)?;
    Ok(schema)
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
            Kind::Untagged(branches) => branches.iter().for_each(|branch| branch.ty.refs(out)),
            Kind::Alias(ty) => ty.refs(out),
            Kind::StringEnum(_) => {}
        }
    }
}

impl Ty {
    /// Whether an optional field of this type keeps an explicit `null` apart
    /// from an omitted key. Only JSON values and presence fields do; a plain
    /// `Option` folds it, as Rust does.
    pub(crate) fn keeps_null(&self) -> bool {
        match self {
            Self::Json | Self::Codec(Codec::Presence, _) => true,
            Self::Nullable(inner) | Self::Codec(_, inner) => inner.keeps_null(),
            _ => false,
        }
    }

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
                let ty = self.ty(value, name, &path)?;
                let mut doc = doc(value);
                if let Ty::Literal(literal) = &ty {
                    let required = format!("Required value: {literal:?}.");
                    doc = Some(doc.map_or(required.clone(), |doc| format!("{doc}\n{required}")));
                }
                fields.push(Field {
                    json_name: key.clone(),
                    ty,
                    required: schema["required"]
                        .as_array()
                        .is_some_and(|r| r.iter().any(|v| v == key)),
                    doc,
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
            let mut overlay = schema
                .as_object()
                .cloned()
                .ok_or_else(|| Self::fail(name, pointer, "expected variant object"))?;
            overlay.remove("$ref");
            schema = target.clone();
            if let Some(object) = schema.as_object_mut() {
                object.remove("description");
            }
            let properties = overlay.remove("properties");
            let required = overlay.remove("required");
            if let Some(properties) = properties {
                for (key, value) in properties.as_object().ok_or_else(|| {
                    Self::fail(name, &format!("{pointer}/properties"), "expected object")
                })? {
                    schema["properties"][key] = value.clone();
                }
            }
            if let Some(required) = required {
                let mut keys = schema["required"].as_array().cloned().unwrap_or_default();
                for key in required.as_array().ok_or_else(|| {
                    Self::fail(name, &format!("{pointer}/required"), "expected array")
                })? {
                    if !keys.contains(key) {
                        keys.push(key.clone());
                    }
                }
                schema["required"] = json!(keys);
            }
            for (key, value) in overlay {
                schema[&key] = value;
            }
        }
        if let Some(variants) = schema["oneOf"].as_array() {
            let overlay = &schema;
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
            if parts.len() == 2
                && parts[1]["type"] == "object"
                && let Some(variants) = parts[0]["oneOf"].as_array()
            {
                let branches = variants
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
            return Ok(Kind::Alias(Ty::Literal(value.to_owned())));
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
                    .map(|value| {
                        value.as_str().map(str::to_owned).ok_or_else(|| {
                            Self::fail(
                                name,
                                &format!("{pointer}/oneOf"),
                                "expected string enum value",
                            )
                        })
                    })
                    .collect::<Result<_>>()?;
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
                    let value = branch["properties"][&tag]["const"]
                        .as_str()
                        .ok_or_else(|| {
                            Self::fail(
                                name,
                                &format!("{path}/properties/{}/const", escape(&tag)),
                                "expected string tag",
                            )
                        })?;
                    variants.push(Variant {
                        tag_value: value.to_owned(),
                        name: variant_name(&branches, branch, &tag, name, &path)?,
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
            let mut names = BTreeSet::new();
            let branches = branches
                .iter()
                .enumerate()
                .map(|(i, branch)| {
                    let ty = self.ty(branch, name, &format!("{pointer}/anyOf/{i}"))?;
                    let branch_name = branch_name(name, &ty);
                    if !names.insert(branch_name.clone()) {
                        return Err(Self::fail(
                            name,
                            pointer,
                            &format!("duplicate untagged branch name {branch_name}"),
                        ));
                    }
                    Ok(Branch {
                        name: branch_name,
                        ty,
                    })
                })
                .collect::<Result<_>>()?;
            return Ok(Kind::Untagged(branches));
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
        let parts = schema["allOf"]
            .as_array()
            .ok_or_else(|| Self::fail(name, &format!("{pointer}/allOf"), "expected array"))?;
        let mut merged = json!({"type":"object", "properties":{}, "required":[]});
        let mut required_keys = Vec::new();
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
                    if !required_keys.contains(key) {
                        required_keys.push(key.clone());
                    }
                }
            }
        }
        merged["required"] = json!(required_keys);
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
            let mut inner = object.clone();
            inner.remove("x-aimux-codec");
            // An extension on an unconstrained Value schema is still explicitly JSON.
            if inner
                .keys()
                .all(|key| ["description", "default", "title"].contains(&key.as_str()))
            {
                return Ok(Ty::Codec(codec, Box::new(Ty::Json)));
            }
            return Ok(Ty::Codec(
                codec,
                Box::new(self.ty(&Value::Object(inner), name, pointer)?),
            ));
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
                inner["type"] = types
                    .iter()
                    .find(|v| *v != "null")
                    .ok_or_else(|| {
                        Self::fail(
                            name,
                            &format!("{pointer}/type"),
                            "nullable union requires a non-null type",
                        )
                    })?
                    .clone();
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
            return Ok(Ty::Nullable(Box::new(
                self.ty(
                    branches
                        .iter()
                        .find(|v| v["type"] != "null")
                        .ok_or_else(|| {
                            Self::fail(
                                name,
                                &format!("{pointer}/anyOf"),
                                "nullable union requires a non-null branch",
                            )
                        })?,
                    name,
                    &format!("{pointer}/anyOf"),
                )?,
            )));
        }
        if let Some(value) = schema["const"].as_str() {
            return Ok(Ty::Literal(value.to_owned()));
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

fn branch_name(union: &str, ty: &Ty) -> String {
    match ty {
        Ty::Ref(name) => name.strip_prefix(union).unwrap_or(name).to_owned(),
        Ty::String | Ty::Literal(_) => "Text".into(),
        Ty::Array(_) => "List".into(),
        Ty::Map(_) => "Map".into(),
        Ty::Bool => "Bool".into(),
        Ty::Int { .. } => "Integer".into(),
        Ty::Float => "Number".into(),
        Ty::Json => "Value".into(),
        Ty::Nullable(inner) | Ty::Codec(_, inner) => branch_name(union, inner),
    }
}

fn variant_name(
    branches: &[Value],
    branch: &Value,
    tag: &str,
    name: &str,
    pointer: &str,
) -> Result<String> {
    let value = &branch["properties"][tag]["const"];
    let siblings: Vec<_> = branches
        .iter()
        .filter(|b| b["properties"][tag]["const"] == *value)
        .collect();
    let outer = pascal(value.as_str().ok_or_else(|| {
        Normalizer::fail(
            name,
            &format!("{pointer}/properties/{}/const", escape(tag)),
            "expected string tag",
        )
    })?);
    if siblings.len() == 1 {
        return Ok(outer);
    }
    let inner = branch["properties"]
        .as_object()
        .ok_or_else(|| Normalizer::fail(name, &format!("{pointer}/properties"), "expected object"))?
        .iter()
        .find(|(key, schema)| {
            key.as_str() != tag
                && schema["const"].is_string()
                && siblings
                    .iter()
                    .all(|b| b["properties"][key.as_str()]["const"].is_string())
                && siblings
                    .iter()
                    .filter_map(|b| b["properties"][key.as_str()]["const"].as_str())
                    .collect::<BTreeSet<_>>()
                    .len()
                    == siblings.len()
        })
        .and_then(|(_, schema)| schema["const"].as_str());
    inner
        .map(|inner| format!("{outer}{}", pascal(inner)))
        .ok_or_else(|| {
            Normalizer::fail(
                name,
                pointer,
                &format!("shared tag {value} requires a literal inner discriminator"),
            )
        })
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
    fn malformed_nullable_and_variant_schemas_report_their_location() {
        for (schema, pointer) in [
            (json!({"type": ["null", "null"]}), "#/type"),
            (
                json!({"anyOf": [{"type": "null"}, {"type": "null"}]}),
                "#/anyOf",
            ),
            (
                json!({"$ref": "#/typeDefs/output/Base", "properties": []}),
                "#/properties",
            ),
            (
                json!({"$ref": "#/typeDefs/output/Base", "properties": {}, "required": {}}),
                "#/required",
            ),
        ] {
            let error = normalize(&json!({"input": {}, "output": {
                "Base": {"type": "object"}, "Example": schema
            }}))
            .unwrap_err();
            assert!(error.starts_with(&format!("Example {pointer}:")), "{error}");
        }
        let error = variant_name(
            &[],
            &json!({"properties": {"type": {"const": 1}}}),
            "type",
            "Example",
            "#/oneOf/0",
        )
        .unwrap_err();
        assert_eq!(
            error,
            "Example #/oneOf/0/properties/type/const: expected string tag"
        );
    }
    #[test]
    fn untagged_branches_are_named_once_from_their_types() {
        let defs = json!({"input":{}, "output": {
            "ToolChoiceMode": {"enum":["auto","none"]},
            "ToolChoice": {"x-aimux-codec":"ordered-untagged", "anyOf":[
                {"$ref":"#/typeDefs/output/ToolChoiceMode"}, {"type":"string"},
                {"type":"array", "items":true}, {"type":"object", "additionalProperties":true},
                {"type":"boolean"}, {"type":"integer"}, {"type":"number"}, true
            ]}
        }});
        let model = normalize(&defs).unwrap();
        let Kind::Untagged(branches) = &model
            .types
            .iter()
            .find(|t| t.name == "ToolChoice")
            .unwrap()
            .kind
        else {
            panic!()
        };
        assert_eq!(
            branches.iter().map(|b| b.name.as_str()).collect::<Vec<_>>(),
            [
                "Mode", "Text", "List", "Map", "Bool", "Integer", "Number", "Value"
            ]
        );
        assert_eq!(
            branch_name("Choice", &Ty::Nullable(Box::new(Ty::Literal("x".into())))),
            "Text"
        );
        assert_eq!(branch_name("Choice", &Ty::Ref("Other".into())), "Other");
        let error = normalize(&json!({"input":{}, "output": {"Choice": {
            "x-aimux-codec":"ordered-untagged", "anyOf":[{"type":"string"}, {"const":"x"}]
        }}}))
        .unwrap_err();
        assert!(
            error.contains("Choice") && error.contains("duplicate untagged branch name Text"),
            "{error}"
        );
    }

    #[test]
    fn shared_outer_tags_use_a_literal_inner_discriminator() {
        let schema = json!({"oneOf":[
            {"type":"object", "properties":{"type":{"const":"source"}, "sourceType":{"const":"url"}}, "required":["type","sourceType"]},
            {"type":"object", "properties":{"type":{"const":"source"}, "sourceType":{"const":"document"}}, "required":["type","sourceType"]},
            {"type":"object", "properties":{"type":{"const":"text"}}, "required":["type"]}
        ]});
        let normalized = model(schema.clone());
        let Kind::Tagged { variants, .. } = &normalized.types[0].kind else {
            panic!()
        };
        assert_eq!(
            variants.iter().map(|v| v.name.as_str()).collect::<Vec<_>>(),
            ["SourceUrl", "SourceDocument", "Text"]
        );
        let mut invalid = schema;
        invalid["oneOf"][0]["properties"]["sourceType"] = json!({"type":"string"});
        let error = normalize(&json!({"input":{}, "output":{"Example":invalid}})).unwrap_err();
        assert!(
            error.contains("Example") && error.contains("literal inner discriminator"),
            "{error}"
        );
    }

    #[test]
    fn literal_fields_stay_inline() {
        let (model, schemas) = normalize_with_schemas(&json!({"input":{}, "output": {
            "Call": {"type":"object", "properties": {
                "type": {"const":"tool-call", "description":"Wire tag."}
            }, "required":["type"]}
        }}))
        .unwrap();
        assert_eq!(schemas.len(), 1);
        let Kind::Struct { fields, .. } = &model.types[0].kind else {
            panic!()
        };
        assert_eq!(fields[0].ty, Ty::Literal("tool-call".into()));
        assert_eq!(
            fields[0].doc.as_deref(),
            Some("Wire tag.\nRequired value: \"tool-call\".")
        );
    }

    #[test]
    fn numbered_siblings_merge_transitively_and_keep_the_first_description() {
        let (_, schemas) = normalize_with_schemas(&json!({"input":{}, "output": {
            "Child":{"type":"object", "properties":{"description":{"type":"string"}}, "description":"First child"},
            "Child2":{"type":"object", "properties":{"description":{"type":"string", "description":"Field docs"}}, "description":"Other child"},
            "Parent":{"type":"object", "properties":{"child":{"$ref":"#/typeDefs/output/Child"}}, "description":"First parent"},
            "Parent2":{"type":"object", "properties":{"child":{"$ref":"#/typeDefs/output/Child2"}}, "description":"Other parent"},
            "Child3":{"type":"object", "properties":{"description":{"type":"boolean"}}}
        }})).unwrap();
        assert_eq!(schemas.len(), 3);
        assert_eq!(schemas["Child"]["description"], "First child");
        assert_eq!(schemas["Parent"]["description"], "First parent");
        assert_eq!(
            schemas["Parent"]["properties"]["child"]["$ref"],
            "Child.json"
        );
        assert!(schemas.contains_key("Child3"));
    }

    #[test]
    fn merged_siblings_follow_contract_splits() {
        let (model, schemas) = normalize_with_schemas(&json!({"input": {
            "Call":{"type":"object", "properties":{"value":{"type":"string"}}},
            "Call2":{"type":"object", "properties":{"value":{"type":"string"}}},
            "Request":{"type":"object", "properties":{"call":{"$ref":"#/typeDefs/input/Call2"}}}
        }, "output": {
            "Call":{"type":"object", "properties":{"value":{"type":"boolean"}}}
        }}))
        .unwrap();
        assert_eq!(model.types.len(), 3);
        assert_eq!(
            schemas["Request"]["properties"]["call"]["$ref"],
            "CallInput.json"
        );
        assert_eq!(schemas["Call"]["properties"]["value"]["type"], "boolean");
        assert_eq!(
            schemas["CallInput"]["properties"]["value"]["type"],
            "string"
        );
    }

    #[test]
    fn path_aliases_resolve_in_fields_and_operation_roots() {
        let defs = json!({"input": {
            "Item":{"type":"object"},
            "_items_":{"type":"array", "items":{"$ref":"#/typeDefs/input/Item"}},
            "_wrapper_":{"$ref":"#/typeDefs/input/_items_"},
            "Request":{"type":"object", "properties":{"items":{"$ref":"#/typeDefs/input/_wrapper_"}}}
        }, "output":{}});
        let (model, schemas) = normalize_with_schemas(&defs).unwrap();
        assert_eq!(model.types.len(), 2);
        assert_eq!(
            schemas["Request"]["properties"]["items"],
            json!({"type":"array", "items":{"$ref":"Item.json"}})
        );
        let definitions = schema_definitions(&defs, &contract_names(&defs).unwrap()).unwrap();
        assert_eq!(
            root_schema(&definitions, "_wrapper_").unwrap(),
            json!({"type":"array", "items":{"$ref":"../types/Item.json"}})
        );
    }

    #[test]
    fn manifest_types_have_names_from_rust() {
        let manifest = aimux_ops::manifest::manifest();
        let defs = &manifest["typeDefs"];
        let names = contract_names(defs).unwrap();
        let model = normalize(defs).unwrap();
        for ty in &model.types {
            assert!(!ty.name.starts_with('_'), "{}", ty.name);
            assert!(
                names.values().any(|name| name == &ty.name),
                "anonymous shape {} needs a Rust type",
                ty.name
            );
        }
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
