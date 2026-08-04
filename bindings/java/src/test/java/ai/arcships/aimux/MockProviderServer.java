package ai.arcships.aimux;

import com.sun.net.httpserver.HttpExchange;
import com.sun.net.httpserver.HttpServer;

import java.io.ByteArrayOutputStream;
import java.io.Closeable;
import java.io.IOException;
import java.io.InputStream;
import java.net.InetSocketAddress;
import java.nio.charset.StandardCharsets;
import java.util.ArrayDeque;
import java.util.Deque;

/**
 * A tiny mock HTTP server that records the last request body and replays a
 * preset response. Listens on a random loopback port (port of the Kotlin
 * binding's {@code MockProviderServer}, built on the JDK's built-in
 * {@link HttpServer} — no extra dependency).
 */
public class MockProviderServer implements Closeable {

    private final HttpServer server;

    /** Last received request body (raw UTF-8 string). */
    private volatile String lastRequestBody = "";

    /** Last received request method (e.g. "POST"). */
    private volatile String lastRequestMethod = "";

    /** Status code to return. */
    private volatile int status = 200;

    /** Value of the Content-Type response header. */
    private volatile String contentType = "application/json";

    /** Body bytes to return (single-response mode; used when the queue is empty). */
    private volatile String responseBody = "";

    /** Response queue — each request pops the next response (FIFO). */
    private final Deque<String> responseQueue = new ArrayDeque<>();

    public MockProviderServer() {
        try {
            server = HttpServer.create(new InetSocketAddress("127.0.0.1", 0), 0);
        } catch (IOException e) {
            throw new RuntimeException("Failed to create mock provider server", e);
        }
        server.createContext("/", this::handle);
        server.start();
    }

    private void handle(HttpExchange exchange) throws IOException {
        // Capture the inbound request body for assertions.
        byte[] reqBytes = readAll(exchange.getRequestBody());
        lastRequestBody = new String(reqBytes, StandardCharsets.UTF_8);
        lastRequestMethod = exchange.getRequestMethod();

        // Pop the next queued response if any, otherwise fall back to the
        // single-response field (backward compatible with existing tests).
        String resp;
        synchronized (responseQueue) {
            resp = responseQueue.isEmpty() ? responseBody : responseQueue.removeFirst();
        }
        byte[] respBytes = resp.getBytes(StandardCharsets.UTF_8);
        exchange.getResponseHeaders().add("Content-Type", contentType);
        // SSE streams use chunked transfer encoding (response length 0) so the
        // client's streaming reader observes a proper end-of-stream. A fixed
        // content length can leave the streaming reader blocked waiting for
        // more data. Plain JSON responses use a fixed length.
        long responseLength =
            contentType.contains("text/event-stream") ? 0L : (long) respBytes.length;
        exchange.sendResponseHeaders(status, responseLength);
        exchange.getResponseBody().write(respBytes);
        exchange.getResponseBody().flush();
        exchange.getResponseBody().close();
        exchange.close();
    }

    private static byte[] readAll(InputStream in) throws IOException {
        ByteArrayOutputStream out = new ByteArrayOutputStream();
        byte[] buf = new byte[8192];
        int n;
        while ((n = in.read(buf)) != -1) {
            out.write(buf, 0, n);
        }
        return out.toByteArray();
    }

    /** Set the list of responses to return (FIFO). */
    public void setResponses(String... responses) {
        synchronized (responseQueue) {
            responseQueue.clear();
            for (String r : responses) {
                responseQueue.addLast(r);
            }
        }
    }

    public int port() {
        return server.getAddress().getPort();
    }

    public String baseUrl() {
        return "http://127.0.0.1:" + port();
    }

    public String lastRequestBody() {
        return lastRequestBody;
    }

    public String lastRequestMethod() {
        return lastRequestMethod;
    }

    public void setStatus(int status) {
        this.status = status;
    }

    public void setContentType(String contentType) {
        this.contentType = contentType;
    }

    public void setResponseBody(String responseBody) {
        this.responseBody = responseBody;
    }

    @Override
    public void close() {
        server.stop(0);
    }
}
