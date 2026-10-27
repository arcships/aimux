use crate::google::get_model_path::get_model_path;

/// upstream: @ai-sdk/google src/get-model-path.test.ts › should pass through model path for models/*
#[test]
fn should_pass_through_model_path_for_models() {
    assert_eq!(get_model_path("models/some-model"), "models/some-model");
}

/// upstream: @ai-sdk/google src/get-model-path.test.ts › should pass through model path for tunedModels/*
#[test]
fn should_pass_through_model_path_for_tuned_models() {
    assert_eq!(
        get_model_path("tunedModels/some-model"),
        "tunedModels/some-model"
    );
}

/// upstream: @ai-sdk/google src/get-model-path.test.ts › should add model path prefix to models without slash
#[test]
fn should_add_model_path_prefix_to_models_without_slash() {
    assert_eq!(get_model_path("some-model"), "models/some-model");
}
