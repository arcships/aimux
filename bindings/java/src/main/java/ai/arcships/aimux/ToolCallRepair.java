package ai.arcships.aimux;

/**
 * Host-side repair for one invalid tool call (RFC-0035) — the aimux equivalent
 * of the AI SDK {@code repairToolCall}.
 *
 * <p>Set it with
 * {@link Types.GenerateTextOptions.Builder#repairToolCall(ToolCallRepair)}. It
 * runs after generation, outside any native call, so it may do anything —
 * including a second {@code generateText} call against a model — to produce the
 * replacement.
 *
 * <p>Contract:
 * <ul>
 *   <li>Return a replacement {@link Types.RawToolCall} (its {@code input} is
 *       raw argument TEXT) to have the library re-parse and re-validate it. If
 *       the replacement is still invalid, the call comes back invalid carrying
 *       a {@code ToolCallRepair} error.</li>
 *   <li>Return {@code null} to leave the call as it was — invalid, with its
 *       original error.</li>
 *   <li>Throw to report a failed attempt: the call comes back invalid with a
 *       {@code ToolCallRepair} error whose cause is the exception's message.</li>
 * </ul>
 *
 * <p>Called at most once per tool call, and never for a valid call or for one
 * made without a tool set.
 */
@FunctionalInterface
public interface ToolCallRepair {

    /**
     * @param context The invalid call, its error, the tool's input schema, and
     *                the tools / messages / instructions the call was generated with.
     * @return The replacement call, or {@code null} to leave it unchanged.
     * @throws Exception to report that the repair attempt failed.
     */
    Types.RawToolCall repair(Types.ToolCallRepairContext context) throws Exception;
}
