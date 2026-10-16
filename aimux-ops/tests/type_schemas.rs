use serde_json::Value;

/// `true` is `serde_json::Value`, the deliberate opaque JSON type; a missing
/// description is a compile error, so only codec names need checking here.
fn check(schema: &Value, root: &str) {
    match schema {
        Value::Object(object) => {
            if let Some(codec) = object.get("x-aimux-codec") {
                assert!(
                    matches!(
                        codec.as_str(),
                        Some(
                            "base64"
                                | "u64-string"
                                | "presence"
                                | "timestamp"
                                | "tagged"
                                | "ordered-untagged"
                        )
                    ),
                    "{root}: unknown codec {codec}"
                );
            }
            object.values().for_each(|child| check(child, root));
        }
        Value::Array(items) => items.iter().for_each(|child| check(child, root)),
        _ => {}
    }
}

#[test]
fn every_op_root_has_a_schema_with_known_codecs() {
    use schemars::generate::{Contract, SchemaSettings};
    let manifest = aimux_ops::manifest::manifest();
    for op in manifest["ops"].as_array().unwrap() {
        let descriptor = aimux_ops::dispatch::descriptor(op["name"].as_str().unwrap()).unwrap();
        for (ty, contract) in [
            (Some(descriptor.request_type), Contract::Deserialize),
            (Some(descriptor.result_type), Contract::Serialize),
            (descriptor.event_type, Contract::Serialize),
            (descriptor.terminal_type, Contract::Serialize),
        ] {
            if let Some(ty) = ty {
                let mut generator = SchemaSettings::default()
                    .with(|s| s.contract = contract)
                    .into_generator();
                check(&(ty.schema)(&mut generator).to_value(), &(ty.id)());
                check(&serde_json::json!(generator.definitions()), &(ty.id)());
            }
        }
    }
    check(&manifest["typeDefs"], "typeDefs");
}
