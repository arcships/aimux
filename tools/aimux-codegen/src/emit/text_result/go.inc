func (v *GenerateTextResult) UnmarshalJSON(data []byte) error {
	type facts GenerateTextResult
	var result facts
	if err := json.Unmarshal(data, &result); err != nil {
		return err
	}
	if len(result.Steps) == 0 {
		return fmt.Errorf("GenerateTextResult requires at least one step")
	}
	if result.OutputValue.Present && result.OutputParsed != nil && !*result.OutputParsed {
		return fmt.Errorf("GenerateTextResult cannot carry output when outputParsed is false")
	}
	*v = GenerateTextResult(result)
	return nil
}

// FinalStep is the last model call in the result.
func (v GenerateTextResult) FinalStep() StepResult { return v.Steps[len(v.Steps)-1] }

func (v GenerateTextResult) Text() string {
	text := ""
	for _, part := range v.FinalStep().Content {
		if part, ok := part.Value.(ContentPartVariantText); ok {
			text += part.Text
		}
	}
	return text
}

func (v GenerateTextResult) Content() []ContentPart {
	out := []ContentPart{}
	for _, step := range v.Steps {
		out = append(out, step.Content...)
	}
	return out
}

// resultParts selects payloads directly from ordered content.
func resultParts[T ContentPartVariant](content []ContentPart) []T {
	out := []T{}
	for _, part := range content {
		if value, ok := part.Value.(T); ok {
			out = append(out, value)
		}
	}
	return out
}

func (v GenerateTextResult) Reasoning() []ContentPart {
	out := []ContentPart{}
	for _, part := range v.FinalStep().Content {
		switch part.Value.(type) {
		case ContentPartVariantReasoning, ContentPartVariantReasoningFile:
			out = append(out, part)
		}
	}
	return out
}

func (v GenerateTextResult) ReasoningText() *string {
	text := ""
	for _, part := range resultParts[ContentPartVariantReasoning](v.FinalStep().Content) {
		text += part.Text
	}
	if text == "" {
		return nil
	}
	return &text
}

func (v GenerateTextResult) Files() []GeneratedFile {
	out := []GeneratedFile{}
	for _, part := range resultParts[ContentPartVariantFile](v.Content()) {
		out = append(out, part.File)
	}
	return out
}

func (v GenerateTextResult) Sources() []ContentPart {
	out := []ContentPart{}
	for _, part := range v.Content() {
		switch part.Value.(type) {
		case ContentPartVariantSourceURL, ContentPartVariantSourceDocument:
			out = append(out, part)
		}
	}
	return out
}

func (v GenerateTextResult) ToolCalls() []ContentPartVariantToolCall {
	return resultParts[ContentPartVariantToolCall](v.Content())
}

func (v GenerateTextResult) toolCallsByDynamic(dynamic bool) []ContentPartVariantToolCall {
	out := []ContentPartVariantToolCall{}
	for _, part := range v.ToolCalls() {
		if (part.Dynamic != nil && *part.Dynamic) == dynamic {
			out = append(out, part)
		}
	}
	return out
}

func (v GenerateTextResult) StaticToolCalls() []ContentPartVariantToolCall {
	return v.toolCallsByDynamic(false)
}

func (v GenerateTextResult) DynamicToolCalls() []ContentPartVariantToolCall {
	return v.toolCallsByDynamic(true)
}

func (v GenerateTextResult) ToolResults() []ContentPartVariantToolResult {
	return resultParts[ContentPartVariantToolResult](v.Content())
}

func (v GenerateTextResult) toolResultsByDynamic(dynamic bool) []ContentPartVariantToolResult {
	out := []ContentPartVariantToolResult{}
	for _, part := range v.ToolResults() {
		if (part.Dynamic != nil && *part.Dynamic) == dynamic {
			out = append(out, part)
		}
	}
	return out
}

func (v GenerateTextResult) StaticToolResults() []ContentPartVariantToolResult {
	return v.toolResultsByDynamic(false)
}

func (v GenerateTextResult) DynamicToolResults() []ContentPartVariantToolResult {
	return v.toolResultsByDynamic(true)
}

func (v GenerateTextResult) FinishReason() FinishReasonUnified { return v.FinalStep().FinishReason }
func (v GenerateTextResult) RawFinishReason() *string          { return v.FinalStep().RawFinishReason }
func (v GenerateTextResult) Usage() LanguageModelUsage         { return v.TotalUsage }
func (v GenerateTextResult) Request() RequestInfo              { return v.FinalStep().Request }
func (v GenerateTextResult) Response() StepResponse            { return v.FinalStep().Response }
func (v GenerateTextResult) ProviderMetadata() map[string]map[string]json.RawMessage {
	return v.FinalStep().ProviderMetadata
}

func (v GenerateTextResult) Warnings() []Warning {
	out := []Warning{}
	for _, step := range v.Steps {
		out = append(out, step.Warnings...)
	}
	return out
}

func (v GenerateTextResult) ResponseMessages() []ModelMessage {
	out := []ModelMessage{}
	for _, step := range v.Steps {
		out = append(out, step.Response.Messages...)
	}
	return out
}

// Output returns the parsed JSON value, the final step's text as a JSON string
// when parsed as plain text, or nil when no output was generated.
func (v GenerateTextResult) Output() json.RawMessage {
	if v.OutputValue.Present {
		if v.OutputValue.Null {
			return json.RawMessage("null")
		}
		return v.OutputValue.Value
	}
	if v.OutputParsed != nil && !*v.OutputParsed {
		return nil
	}
	value, _ := json.Marshal(v.Text())
	return value
}
