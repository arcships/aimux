package ai.arcships.aimux;

import ai.arcships.aimux.types.*;

import ai.arcships.aimux.generated.AimuxOps;

import com.fasterxml.jackson.databind.JsonNode;

/** Provider resource. Provider configuration is data, passed unchanged to provider.create. */
public final class ProviderHandle extends NativeHandle {
    ProviderHandle(Aimux aimux, long id) { super(aimux, id); }

    private long modelId(String method, String modelId) {
        ProviderModelParams request = ProviderModelParams.builder()
            .method(method).modelId(java.util.Objects.requireNonNull(modelId, "modelId")).build();
        return AimuxResult.handle(call(AimuxOps.OP_PROVIDER_MODEL, DtoJson.stringify(request)));
    }

    public Model languageModel(String modelId) { return new Model(aimux, modelId("languageModel", modelId)); }
    public EmbeddingModel embeddingModel(String modelId) { return new EmbeddingModel(aimux, modelId("embeddingModel", modelId)); }
    public ImageModel imageModel(String modelId) { return new ImageModel(aimux, modelId("imageModel", modelId)); }
    public SpeechModel speechModel(String modelId) { return new SpeechModel(aimux, modelId("speechModel", modelId)); }
    public TranscriptionModel transcriptionModel(String modelId) { return new TranscriptionModel(aimux, modelId("transcriptionModel", modelId)); }
    public RerankingModel rerankingModel(String modelId) { return new RerankingModel(aimux, modelId("rerankingModel", modelId)); }
    public VideoModel videoModel(String modelId) { return new VideoModel(aimux, modelId("videoModel", modelId)); }
    public SearchModel searchModel(String modelId) { return new SearchModel(aimux, modelId("searchModel", modelId)); }
    public DecisionModel decisionModel(String modelId) { return new DecisionModel(aimux, modelId("decision", modelId)); }
    public Files files() { return new Files(aimux, AimuxResult.handle(call(AimuxOps.OP_PROVIDER_FILES, "{}"))); }
    public JsonNode invoke(String paramsJson) { return call(AimuxOps.OP_PROVIDER_INVOKE, paramsJson); }
}
