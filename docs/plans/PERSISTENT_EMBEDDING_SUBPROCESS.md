# Plan: Persistent Embedding Subprocess

## Problem
`bin/embed` takes ~20s per call due to Python/model startup overhead. Actual embedding is ~100ms.

**Breakdown:**
- Import time: ~13s
- Model load time: ~5.5s
- Actual encoding: ~0.1s

## Solution
Keep a long-lived Python subprocess with the model loaded. Communicate via stdin/stdout JSON-lines. Respawn on crash.

## Changes

### 1. Create `bin/embed-server` (Python)
New persistent script that:
- Loads model once on startup
- Reads JSON-lines from stdin: `{"texts": ["hello"]}\n`
- Writes JSON-lines to stdout: `{"embeddings": [[...]], "dimensions": 384}\n`
- Supports health check: `{"ping": true}` → `{"pong": true}`
- Flushes stdout after every response (critical!)
- Exits cleanly when stdin closes
- Writes "Ready for requests" to stderr when model is loaded

```python
#!/usr/bin/env python3
"""Persistent embedding server for Recollect.

Reads JSON-lines from stdin, writes JSON-lines to stdout.
Model is loaded once on startup and kept warm.
"""
import sys
import json
import signal
from sentence_transformers import SentenceTransformer

MODEL_NAME = "sentence-transformers/all-MiniLM-L6-v2"
DIMENSIONS = 384

def main():
    sys.stderr.write(f"Loading model {MODEL_NAME}...\n")
    sys.stderr.flush()
    model = SentenceTransformer(MODEL_NAME)
    sys.stderr.write("Ready for requests.\n")
    sys.stderr.flush()

    def shutdown(signum, frame):
        sys.exit(0)

    signal.signal(signal.SIGTERM, shutdown)
    signal.signal(signal.SIGINT, shutdown)

    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue

        try:
            request = json.loads(line)
        except json.JSONDecodeError as e:
            sys.stdout.write(json.dumps({"error": f"Invalid JSON: {e}"}) + "\n")
            sys.stdout.flush()
            continue

        if request.get("ping"):
            sys.stdout.write(json.dumps({"pong": True}) + "\n")
            sys.stdout.flush()
            continue

        texts = request.get("texts", [])
        if not texts:
            response = {"embeddings": [], "dimensions": DIMENSIONS}
        else:
            embeddings = model.encode(texts, normalize_embeddings=True)
            response = {
                "embeddings": [emb.tolist() for emb in embeddings],
                "dimensions": DIMENSIONS
            }

        sys.stdout.write(json.dumps(response) + "\n")
        sys.stdout.flush()

if __name__ == "__main__":
    main()
```

### 2. Modify `lib/recollect/embedding_client.rb`
Replace spawn-per-call with persistent subprocess:

```ruby
# frozen_string_literal: true

require "json"
require "open3"
require "timeout"

module Recollect
  class EmbeddingClient
    class EmbeddingError < StandardError; end

    STARTUP_TIMEOUT = 120  # Model loading can take time
    REQUEST_TIMEOUT = 30   # Individual requests should be fast

    def initialize(config: Recollect.config)
      @python_path = config.python_path
      @script_path = config.embed_server_script_path.to_s
      @mutex = Mutex.new
      @stdin = nil
      @stdout = nil
      @stderr = nil
      @wait_thread = nil
    end

    def embed(text)
      embed_batch([text]).first
    end

    def embed_batch(texts)
      return [] if texts.empty?

      @mutex.synchronize do
        ensure_process_running!
        send_request({ texts: texts })
      end
    end

    def healthy?
      @mutex.synchronize do
        return false unless process_alive?
        send_request({ ping: true }, timeout: 5)
        true
      end
    rescue
      false
    end

    def shutdown
      @mutex.synchronize do
        stop_process
      end
    end

    private

    def ensure_process_running!
      return if process_alive?
      start_process
    end

    def process_alive?
      @wait_thread&.alive?
    end

    def start_process
      stop_process if @wait_thread

      @stdin, @stdout, @stderr, @wait_thread = Open3.popen3(
        @python_path, @script_path
      )

      wait_for_ready
    end

    def wait_for_ready
      Timeout.timeout(STARTUP_TIMEOUT) do
        loop do
          line = @stderr.gets
          break if line.nil?
          break if line.include?("Ready for requests")
        end
      end
    rescue Timeout::Error
      stop_process
      raise EmbeddingError, "Embedding process startup timed out after #{STARTUP_TIMEOUT}s"
    end

    def stop_process
      @stdin&.close rescue nil
      @stdout&.close rescue nil
      @stderr&.close rescue nil

      if @wait_thread&.alive?
        Process.kill("TERM", @wait_thread.pid) rescue nil
        @wait_thread.join(5) rescue nil
      end

      @stdin = @stdout = @stderr = @wait_thread = nil
    end

    def send_request(request, timeout: REQUEST_TIMEOUT)
      raise EmbeddingError, "Process not running" unless process_alive?

      json_line = JSON.generate(request) + "\n"
      @stdin.write(json_line)
      @stdin.flush

      response_line = Timeout.timeout(timeout) { @stdout.gets }

      raise EmbeddingError, "Process died (stdout closed)" if response_line.nil?

      result = JSON.parse(response_line)
      raise EmbeddingError, result["error"] if result["error"]

      result["pong"] ? true : result["embeddings"]
    rescue Timeout::Error
      stop_process
      raise EmbeddingError, "Embedding request timed out after #{timeout}s"
    rescue Errno::EPIPE
      stop_process
      raise EmbeddingError, "Embedding process died (broken pipe)"
    rescue JSON::ParserError => e
      raise EmbeddingError, "Invalid response from embedding process: #{e.message}"
    end
  end
end
```

Key behaviors:
- Lazy start on first request
- Mutex for thread safety
- Detect dead process via `@wait_thread.alive?`
- Respawn automatically on next request after crash
- `shutdown` method for clean teardown

### 3. Update `lib/recollect/config.rb`
Add `embed_server_script_path` attribute:

```ruby
attr_accessor :embed_server_script_path

# In initialize:
@embed_server_script_path = Recollect.root.join("bin", "embed-server")
```

### 4. Update tests
Modify `test/recollect/embedding_client_test.rb`:
- Test that subsequent requests are fast (process reuse)
- Test crash recovery (kill process, next request should work)
- Test shutdown method
- Test thread safety

## Files to modify
| File | Action |
|------|--------|
| `bin/embed-server` | CREATE |
| `lib/recollect/embedding_client.rb` | MODIFY |
| `lib/recollect/config.rb` | MODIFY |
| `test/recollect/embedding_client_test.rb` | MODIFY |

## Gotchas
1. **Buffering**: Python must `sys.stdout.flush()` after every response or Ruby will hang
2. **Zombie processes**: Python exits when stdin closes (Ruby parent dies)
3. **First request**: Still ~18s for initial model load, but only once per server restart
4. **Thread safety**: Mutex protects subprocess state
5. **stderr reading**: Read stderr in `wait_for_ready` but don't block on it during normal operation

## Expected Performance
- First request: ~18s (model loading)
- Subsequent requests: ~100ms
