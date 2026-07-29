import XCTest
@testable import Aimux

final class AimuxTests: XCTestCase {

    func testModelCreation() throws {
        // Even with a fake key, the provider should construct
        // (key is validated on first API call, not construction).
        let model = try Model.openai(apiKey: "sk-test-fake-key", modelId: "gpt-4o-mini")
        XCTAssertNotNil(model)
    }

    func testAnthropicModelCreation() throws {
        let model = try Model.anthropic(
            apiKey: "sk-ant-test-fake-key",
            modelId: "claude-3-5-sonnet-20241022"
        )
        XCTAssertNotNil(model)
    }

    func testInvalidPromptThrows() throws {
        let model = try Model.openai(apiKey: "sk-test-fake-key", modelId: "gpt-4o-mini")
        XCTAssertThrowsError(try model.generateText(prompt: "{invalid json}"))
    }

    func testStreamTextReturnsAsyncSequence() throws {
        let model = try Model.openai(apiKey: "sk-test-fake-key", modelId: "gpt-4o-mini")
        let stream = model.streamTextAsync(prompt: "\"hello\"")
        // AsyncStream is created — we don't consume it (would need network).
        XCTAssertNotNil(stream)
    }
}
