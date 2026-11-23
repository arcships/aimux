
    public StepResult getFinalStep() { return steps.get(steps.size() - 1); }
    public String getText() {
        return getFinalStep().getContent().stream().filter(ContentPart.Text.class::isInstance)
            .map(ContentPart::getText).collect(java.util.stream.Collectors.joining());
    }
    public List<ContentPart> getContent() {
        return steps.stream().flatMap(step -> step.getContent().stream())
            .collect(java.util.stream.Collectors.toList());
    }
    public List<ReasoningPart> getReasoning() {
        List<ReasoningPart> reasoning = new ArrayList<>();
        for (ContentPart part : getFinalStep().getContent()) {
            if (part instanceof ContentPart.Reasoning) {
                reasoning.add(new ReasoningPart.Reasoning(part.getProviderMetadata(), part.getText()));
            } else if (part instanceof ContentPart.ReasoningFile) {
                reasoning.add(new ReasoningPart.ReasoningFile(part.getFile(), part.getProviderMetadata()));
            }
        }
        return reasoning;
    }
    public String getReasoningText() {
        String text = getFinalStep().getContent().stream().filter(ContentPart.Reasoning.class::isInstance)
            .map(ContentPart::getText).collect(java.util.stream.Collectors.joining());
        return text.isEmpty() ? null : text;
    }
    private <T extends ContentPart> java.util.stream.Stream<T> parts(Class<T> type) {
        return steps.stream().flatMap(step -> step.getContent().stream())
            .filter(type::isInstance).map(type::cast);
    }
    public List<GeneratedFile> getFiles() {
        return parts(ContentPart.File.class).map(ContentPart::getFile)
            .collect(java.util.stream.Collectors.toList());
    }
    public List<TypedSource> getSources() {
        List<TypedSource> sources = new ArrayList<>();
        parts(ContentPart.SourceUrlGroup.class).forEach(part -> {
            if (part instanceof ContentPart.SourceUrl) {
                ContentPart.SourceUrl url = (ContentPart.SourceUrl) part;
                sources.add(new TypedSource.Url(url.getId(), url.getProviderMetadata(), url.getTitle(),
                    url.getType(), url.getUrl()));
            } else if (part instanceof ContentPart.SourceDocument) {
                ContentPart.SourceDocument doc = (ContentPart.SourceDocument) part;
                sources.add(new TypedSource.Document(doc.getFilename(), doc.getId(), doc.getMediaType(),
                    doc.getProviderMetadata(), doc.getTitle(), doc.getType()));
            }
        });
        return sources;
    }
    public List<TypedToolCall> getToolCalls() {
        return parts(ContentPart.ToolCallVariant.class)
            .map(part -> new TypedToolCall(part.getDynamic(), part.getError(), part.getInput(), part.getInvalid(),
                part.getProviderExecuted(), part.getProviderMetadata(), part.getToolCallId(), part.getToolName(),
                part.getType()))
            .collect(java.util.stream.Collectors.toList());
    }
    public List<TypedToolCall> getStaticToolCalls() {
        return getToolCalls().stream().filter(part -> !Boolean.TRUE.equals(part.getDynamic()))
            .collect(java.util.stream.Collectors.toList());
    }
    public List<TypedToolCall> getDynamicToolCalls() {
        return getToolCalls().stream().filter(part -> Boolean.TRUE.equals(part.getDynamic()))
            .collect(java.util.stream.Collectors.toList());
    }
    public List<TaggedToolResult> getToolResults() {
        return parts(ContentPart.ToolResultVariant.class)
            .map(part -> new TaggedToolResult(part.getDynamic(), part.getInput(), part.getOutput(),
                part.getPreliminary(), part.getProviderExecuted(), part.getProviderMetadata(), part.getToolCallId(),
                part.getToolName(), part.getType()))
            .collect(java.util.stream.Collectors.toList());
    }
    public List<TaggedToolResult> getStaticToolResults() {
        return getToolResults().stream().filter(part -> !Boolean.TRUE.equals(part.getDynamic()))
            .collect(java.util.stream.Collectors.toList());
    }
    public List<TaggedToolResult> getDynamicToolResults() {
        return getToolResults().stream().filter(part -> Boolean.TRUE.equals(part.getDynamic()))
            .collect(java.util.stream.Collectors.toList());
    }
    public FinishReasonUnified getFinishReason() { return getFinalStep().getFinishReason(); }
    public String getRawFinishReason() { return getFinalStep().getRawFinishReason(); }
    public LanguageModelUsage getUsage() { return totalUsage; }
    public RequestInfo getRequest() { return getFinalStep().getRequest(); }
    public StepResponse getResponse() { return getFinalStep().getResponse(); }
    public Map<String, Map<String, JsonNode>> getProviderMetadata() { return getFinalStep().getProviderMetadata(); }
    public List<Warning> getWarnings() {
        return steps.stream().flatMap(step -> step.getWarnings().stream()).collect(java.util.stream.Collectors.toList());
    }
    public List<ModelMessage> getResponseMessages() {
        return steps.stream().flatMap(step -> step.getResponse().getMessages().stream())
            .collect(java.util.stream.Collectors.toList());
    }
