# Code Review Findings: Recollect (Ruby MCP Memory)

**Date:** Wednesday, January 14, 2026
**Scope:** Logical errors, inconsistencies, and security issues for local-first usage.

---

## 1. Security & Safety

### SQL Injection
*   **Status:** **Pass**
*   **Finding:** The application consistently uses parameterized queries (`?` placeholders) in the `Database` class. FTS5 queries are manually escaped (`gsub('"', '""')`), which is the correct approach for SQLite FTS to prevent syntax errors and injection within the virtual table query language.

### Path Traversal
*   **Status:** **Pass**
*   **Finding:** `DatabaseManager#sanitize_project_name` strictly filters project names using `[^a-zA-Z0-9_]`. This effectively prevents directory traversal attacks (e.g., `../../`).
*   **Observation:** The strict filter prevents using hyphens (`-`) in project names. It is recommended to update the regex to `/[^a-zA-Z0-9_\-]/`.

### Command Injection
*   **Status:** **Pass**
*   **Finding:** `EmbeddingClient` uses `Open3.popen3` with an argument list (`@python_path`, `@script_path`), which avoids shell interpolation and prevents command injection.

---

## 2. Logical Errors & Robustness

### Fragile Tag Search (High Priority) 
*   **Location:** `lib/recollect/database.rb:139` (`search_by_tags`)
*   **Issue:** The query uses `LIKE "%\"#{tag}\"%"` to match tags stored in a JSON array string.
*   **Risk:** This fails for tags containing special characters (quotes, backslashes) that are escaped by `JSON.generate`.
    *   *Example:* A tag `bug"fix` is stored as `["bug\"fix"]`. The search pattern becomes `%"bug\"fix"%`, which will not match.
*   **Recommendation:** Use SQLite's `json_each` function for robust matching:
    ```sql
    WHERE EXISTS (SELECT 1 FROM json_each(tags) WHERE value = ?)
    ```

### Unbounded Background Queue
*   **Location:** `lib/recollect/embedding_worker.rb`
*   **Issue:** The `@queue` (Ruby `Queue`) is unbounded.
*   **Risk:** If the Python embedding process hangs or is significantly slower than the incoming request rate, the queue will grow indefinitely, potentially leading to Out-of-Memory (OOM) failures.
*   **Recommendation:** Use `SizedQueue.new(MAX_SIZE)` and implement a strategy for handling a full queue (e.g., blocking or dropping).

### Embedding Correlation Risk
*   **Location:** `lib/recollect/embedding_worker.rb`
*   **Issue:** `process_batch` uses `batch.zip(embeddings)` to pair results.
*   **Risk:** If the `EmbeddingClient` returns a different number of results than expected (due to an error or internal filtering), `zip` will incorrectly pair embeddings with the wrong memory records.
*   **Recommendation:** Ensure the Python script returns a 1:1 mapping or includes an identifier in the response.

---

## 3. Configuration & Consistency

### Hardcoded Library Paths
*   **Location:** `lib/recollect/config.rb`
*   **Issue:** `vec_extension_path` uses a hardcoded list of filesystem paths.
*   **Recommendation:** Add support for an environment variable (e.g., `RECOLLECT_SQLITE_VEC_PATH`) to allow users to specify custom installation paths without modifying the code.

### Vector Availability Misreporting
*   **Finding:** `Config#vectors_available?` checks for the *existence* of the extension file, while `Database#load_vector_extension` handles the actual loading.
*   **Risk:** If the extension exists but fails to load (e.g., due to missing dependencies like `libgomp`), the UI/CLI might report vectors as "enabled" while the database has silently disabled them.
*   **Recommendation:** Ensure the status check accurately reflects the state of the active database connection.

---

## 4. Summary of Recommended Actions

1.  **Refactor Tag Search:** Replace `LIKE` with `json_each` in `Database#search_by_tags`.
2.  **Harden Background Worker:** Switch to `SizedQueue` and add length validation in `process_batch`.
3.  **Update Sanitization:** Allow hyphens in project names.
4.  **Enhance Configuration:** Add `RECOLLECT_SQLITE_VEC_PATH` environment variable support.
