package ai.arcships.aimux;

import ai.arcships.aimux.types.DtoTypes;
import ai.arcships.aimux.types.WireJson;
import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.node.ObjectNode;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;

/** Round-trips every indexed Rust vector through its generated Java DTO. */
public final class DtoVectors {
    public static void main(String[] args) throws Exception {
        Path directory = Paths.get(args[0]);
        JsonNode index = WireJson.MAPPER.readTree(directory.resolve("index.json").toFile());
        for (JsonNode entry : index) {
            ObjectNode result = WireJson.MAPPER.createObjectNode();
            String file = entry.get("file").textValue();
            result.put("file", file);
            try {
                JsonNode value = WireJson.parse(new String(Files.readAllBytes(directory.resolve(file)), StandardCharsets.UTF_8));
                result.set("value", DtoTypes.roundTrip(entry.get("type").textValue(), value));
                result.put("ok", true);
            } catch (Exception error) {
                result.put("ok", false);
                result.put("error", error.toString());
            }
            System.out.println(result);
        }
    }
}
