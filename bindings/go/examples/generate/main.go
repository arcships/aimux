package main

import (
	"context"
	"fmt"
	"log"
	"os"

	"github.com/arcships/aimux/bindings/go"
)

func main() {
	ctx := context.Background()
	provider, err := aimux.Create(ctx, nil, "provider.create", map[string]any{
		"package": "openai", "settings": map[string]any{"apiKey": os.Getenv("OPENAI_API_KEY")},
	})
	if err != nil {
		log.Fatal(err)
	}
	defer provider.Close()
	model, err := aimux.Create(ctx, provider, "provider.model", map[string]any{
		"method": "languageModel", "modelId": "gpt-4o-mini",
	})
	if err != nil {
		log.Fatal(err)
	}
	defer model.Close()
	result, err := aimux.CallInto[aimux.GenerateTextResult](ctx, model, "text.generate", map[string]any{
		"prompt": "Say hello in one sentence.",
	})
	if err != nil {
		log.Fatal(err)
	}
	fmt.Println(result.Text)
}
