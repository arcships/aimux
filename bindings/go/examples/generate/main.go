package main

import (
	"context"
	"fmt"
	"log"
	"os"

	"github.com/arcships/aimux/bindings/go"
	"github.com/arcships/aimux/bindings/go/generated"
)

func main() {
	ctx := context.Background()
	provider, err := aimux.Create(ctx, nil, generated.OP_PROVIDER_CREATE, map[string]any{
		"package": "openai", "settings": map[string]any{"apiKey": os.Getenv("OPENAI_API_KEY")},
	})
	if err != nil {
		log.Fatal(err)
	}
	defer provider.Close()
	model, err := aimux.Create(ctx, provider, generated.OP_PROVIDER_MODEL, map[string]any{
		"method": "languageModel", "modelId": "gpt-4o-mini",
	})
	if err != nil {
		log.Fatal(err)
	}
	defer model.Close()
	result, err := aimux.CallInto[aimux.GenerateTextResult](ctx, model, generated.OP_TEXT_GENERATE, map[string]any{
		"prompt": "Say hello in one sentence.",
	})
	if err != nil {
		log.Fatal(err)
	}
	fmt.Println(result.Text)
}
