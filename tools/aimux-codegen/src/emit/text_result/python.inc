
    @property
    def final_step(self) -> StepResult:
        return self.steps[-1]

    @property
    def text(self) -> str:
        return "".join(part.text for part in self.final_step.content if isinstance(part, ContentPartText))

    @property
    def content(self) -> list[ContentPart]:
        return [part for step in self.steps for part in step.content]

    @property
    def reasoning(self) -> list[ContentPartReasoning | ContentPartReasoningFile]:
        return [part for part in self.final_step.content
                if isinstance(part, (ContentPartReasoning, ContentPartReasoningFile))]

    @property
    def reasoning_text(self) -> Optional[str]:
        return "".join(part.text for part in self.reasoning if isinstance(part, ContentPartReasoning)) or None

    @property
    def files(self) -> list[GeneratedFile]:
        return [part.file for part in self.content if isinstance(part, ContentPartFile)]

    @property
    def sources(self) -> list[ContentPartSourceUrl | ContentPartSourceDocument]:
        return [part for part in self.content if isinstance(part, (ContentPartSourceUrl, ContentPartSourceDocument))]

    @property
    def tool_calls(self) -> list[ContentPartToolCall]:
        return [part for part in self.content if isinstance(part, ContentPartToolCall)]

    @property
    def static_tool_calls(self) -> list[ContentPartToolCall]:
        return [part for part in self.tool_calls if part.dynamic is not True]

    @property
    def dynamic_tool_calls(self) -> list[ContentPartToolCall]:
        return [part for part in self.tool_calls if part.dynamic is True]

    @property
    def tool_results(self) -> list[ContentPartToolResult]:
        return [part for part in self.content if isinstance(part, ContentPartToolResult)]

    @property
    def static_tool_results(self) -> list[ContentPartToolResult]:
        return [part for part in self.tool_results if part.dynamic is not True]

    @property
    def dynamic_tool_results(self) -> list[ContentPartToolResult]:
        return [part for part in self.tool_results if part.dynamic is True]

    @property
    def finish_reason(self) -> FinishReasonUnified:
        return self.final_step.finish_reason

    @property
    def raw_finish_reason(self) -> Optional[str]:
        return self.final_step.raw_finish_reason

    @property
    def usage(self) -> LanguageModelUsage:
        return self.total_usage

    @property
    def warnings(self) -> list[Warning]:
        return [warning for step in self.steps for warning in step.warnings]

    @property
    def request(self) -> RequestInfo:
        return self.final_step.request

    @property
    def response(self) -> StepResponse:
        return self.final_step.response

    @property
    def provider_metadata(self) -> Optional[dict[str, dict[str, Any]]]:
        return self.final_step.provider_metadata

    @property
    def response_messages(self) -> list[ModelMessage]:
        return [message for step in self.steps for message in step.response.messages]

    @model_validator(mode="after")
    def _validate_output_state(self) -> "GenerateTextResult":
        if "output_value" in self.model_fields_set and self.output_parsed is False:
            raise ValueError("GenerateTextResult cannot carry output when outputParsed is false")
        return self

    @property
    def output(self) -> Any:
        if "output_value" in self.model_fields_set:
            return self.output_value
        if self.output_parsed is False:
            return None
        return self.text
