# Recency-Based Search Ranking Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add configurable recency boost to search results that prefers newer memories using exponential decay.

**Architecture:** Create a `RecencyRanker` class that applies multiplicative exponential decay to search scores. Configure via environment variables. Apply after relevance scoring in both FTS-only and hybrid search paths. Disabled by default (aging_factor=0).

**Tech Stack:** Ruby, SQLite FTS5, existing HybridSearchRanker pattern

---

## Formula Reference

```
effective_recency = 1 - aging_factor + (aging_factor * e^(-ln(2) * age_in_days / half_life_days))
final_score = relevance_score * effective_recency
```

- `aging_factor = 0.0`: No recency effect (disabled)
- `aging_factor = 1.0`: Full recency effect
- At `half_life_days`: recency factor = 0.5 (when aging_factor=1.0)

---

## Task 1: Add Configuration Options

**Files:**
- Modify: `lib/recollect/config.rb:8-10` (attr_accessor)
- Modify: `lib/recollect/config.rb:12-14` (constants)
- Modify: `lib/recollect/config.rb:28-30` (initialize)
- Modify: `lib/recollect/config.rb:36` (predicate method)
- Test: `test/recollect/config_test.rb`

**Step 1: Write failing tests for config**

Add to `test/recollect/config_test.rb`:

```ruby
def test_default_recency_aging_factor
  assert_in_delta 0.0, @config.recency_aging_factor
end

def test_default_recency_half_life_days
  assert_in_delta 30.0, @config.recency_half_life_days
end

def test_recency_disabled_by_default
  refute @config.recency_enabled?
end

def test_recency_aging_factor_from_env
  ENV["RECOLLECT_RECENCY_AGING_FACTOR"] = "0.7"
  config = Recollect::Config.new

  assert_in_delta 0.7, config.recency_aging_factor
ensure
  ENV.delete("RECOLLECT_RECENCY_AGING_FACTOR")
end

def test_recency_aging_factor_clamped_high
  ENV["RECOLLECT_RECENCY_AGING_FACTOR"] = "1.5"
  config = Recollect::Config.new

  assert_in_delta 1.0, config.recency_aging_factor
ensure
  ENV.delete("RECOLLECT_RECENCY_AGING_FACTOR")
end

def test_recency_aging_factor_clamped_low
  ENV["RECOLLECT_RECENCY_AGING_FACTOR"] = "-0.5"
  config = Recollect::Config.new

  assert_in_delta 0.0, config.recency_aging_factor
ensure
  ENV.delete("RECOLLECT_RECENCY_AGING_FACTOR")
end

def test_recency_half_life_days_from_env
  ENV["RECOLLECT_RECENCY_HALF_LIFE_DAYS"] = "14"
  config = Recollect::Config.new

  assert_in_delta 14.0, config.recency_half_life_days
ensure
  ENV.delete("RECOLLECT_RECENCY_HALF_LIFE_DAYS")
end

def test_recency_enabled_when_aging_factor_positive
  ENV["RECOLLECT_RECENCY_AGING_FACTOR"] = "0.5"
  config = Recollect::Config.new

  assert config.recency_enabled?
ensure
  ENV.delete("RECOLLECT_RECENCY_AGING_FACTOR")
end
```

**Step 2: Run tests to verify they fail**

Run: `bundle exec ruby -Itest test/recollect/config_test.rb -n /recency/`
Expected: FAIL with "undefined method `recency_aging_factor'"

**Step 3: Implement config options**

In `lib/recollect/config.rb`:

Add to attr_accessor (line 8-10):
```ruby
attr_accessor :data_dir, :host, :port, :max_results,
  :enable_vectors, :vector_dimensions, :embed_server_script_path,
  :log_wiredumps, :max_vector_distance,
  :recency_aging_factor, :recency_half_life_days
```

Add constants after line 14:
```ruby
DEFAULT_RECENCY_AGING_FACTOR = 0.0
DEFAULT_RECENCY_HALF_LIFE_DAYS = 30.0
```

Add to initialize method (after line 30, before `ensure_directories!`):
```ruby
# Recency ranking configuration
@recency_aging_factor = ENV.fetch("RECOLLECT_RECENCY_AGING_FACTOR",
  DEFAULT_RECENCY_AGING_FACTOR).to_f.clamp(0.0, 1.0)
@recency_half_life_days = ENV.fetch("RECOLLECT_RECENCY_HALF_LIFE_DAYS",
  DEFAULT_RECENCY_HALF_LIFE_DAYS).to_f
```

Add predicate method (after line 36):
```ruby
def recency_enabled?
  @recency_aging_factor > 0.0
end
```

**Step 4: Run tests to verify they pass**

Run: `bundle exec ruby -Itest test/recollect/config_test.rb -n /recency/`
Expected: All 8 tests PASS

**Step 5: Commit**

```bash
git add lib/recollect/config.rb test/recollect/config_test.rb
git commit -m "feat(config): add recency ranking configuration options"
```

---

## Task 2: Create RecencyRanker Class - Core Calculation

**Files:**
- Create: `lib/recollect/recency_ranker.rb`
- Create: `test/recollect/recency_ranker_test.rb`

**Step 1: Write failing test for calculate_recency_factor**

Create `test/recollect/recency_ranker_test.rb`:

```ruby
# frozen_string_literal: true

require "test_helper"

class RecencyRankerTest < Recollect::TestCase
  def setup
    super
    @reference_time = Time.parse("2025-01-15T12:00:00Z")
  end

  def test_recency_factor_for_current_time
    ranker = Recollect::RecencyRanker.new(
      aging_factor: 1.0,
      half_life_days: 30,
      reference_time: @reference_time
    )

    # Same time = factor of 1.0
    factor = ranker.calculate_recency_factor("2025-01-15T12:00:00Z")
    assert_in_delta 1.0, factor, 0.001
  end

  def test_recency_factor_at_half_life
    ranker = Recollect::RecencyRanker.new(
      aging_factor: 1.0,
      half_life_days: 14,
      reference_time: @reference_time
    )

    # 14 days ago = factor of 0.5
    factor = ranker.calculate_recency_factor("2025-01-01T12:00:00Z")
    assert_in_delta 0.5, factor, 0.01
  end

  def test_recency_factor_at_two_half_lives
    ranker = Recollect::RecencyRanker.new(
      aging_factor: 1.0,
      half_life_days: 7,
      reference_time: @reference_time
    )

    # 14 days = 2 half-lives = factor of 0.25
    factor = ranker.calculate_recency_factor("2025-01-01T12:00:00Z")
    assert_in_delta 0.25, factor, 0.01
  end

  def test_recency_factor_with_partial_aging
    ranker = Recollect::RecencyRanker.new(
      aging_factor: 0.5,
      half_life_days: 14,
      reference_time: @reference_time
    )

    # 14 days with aging_factor=0.5:
    # effective = 1 - 0.5 + (0.5 * 0.5) = 0.75
    factor = ranker.calculate_recency_factor("2025-01-01T12:00:00Z")
    assert_in_delta 0.75, factor, 0.01
  end

  def test_recency_factor_with_zero_aging
    ranker = Recollect::RecencyRanker.new(
      aging_factor: 0.0,
      half_life_days: 14,
      reference_time: @reference_time
    )

    # aging_factor=0 means no decay, always 1.0
    factor = ranker.calculate_recency_factor("2025-01-01T12:00:00Z")
    assert_in_delta 1.0, factor, 0.001
  end

  def test_handles_nil_timestamp
    ranker = Recollect::RecencyRanker.new(
      aging_factor: 1.0,
      half_life_days: 30,
      reference_time: @reference_time
    )

    factor = ranker.calculate_recency_factor(nil)
    assert_in_delta 1.0, factor, 0.001
  end

  def test_handles_invalid_timestamp
    ranker = Recollect::RecencyRanker.new(
      aging_factor: 1.0,
      half_life_days: 30,
      reference_time: @reference_time
    )

    factor = ranker.calculate_recency_factor("not-a-timestamp")
    assert_in_delta 1.0, factor, 0.001
  end

  def test_handles_time_object
    ranker = Recollect::RecencyRanker.new(
      aging_factor: 1.0,
      half_life_days: 14,
      reference_time: @reference_time
    )

    timestamp = Time.parse("2025-01-01T12:00:00Z")
    factor = ranker.calculate_recency_factor(timestamp)
    assert_in_delta 0.5, factor, 0.01
  end
end
```

**Step 2: Run tests to verify they fail**

Run: `bundle exec ruby -Itest test/recollect/recency_ranker_test.rb`
Expected: FAIL with "cannot load such file -- recollect/recency_ranker"

**Step 3: Implement RecencyRanker core**

Create `lib/recollect/recency_ranker.rb`:

```ruby
# frozen_string_literal: true

require "time"

module Recollect
  # Applies recency-based score adjustment to search results.
  # Uses exponential decay with configurable half-life.
  #
  # Formula:
  #   effective_recency = 1 - aging_factor + (aging_factor * exp_decay)
  #   exp_decay = e^(-lambda * age_in_days)
  #   lambda = ln(2) / half_life_days
  #
  # When aging_factor=0, no recency effect.
  # When aging_factor=1, full recency effect.
  class RecencyRanker
    LN_2 = Math.log(2)
    SECONDS_PER_DAY = 86400.0

    def initialize(aging_factor:, half_life_days:, reference_time: nil)
      @aging_factor = aging_factor.to_f.clamp(0.0, 1.0)
      @half_life_days = [half_life_days.to_f, 0.1].max # Avoid division by zero
      @lambda = LN_2 / @half_life_days
      @reference_time = reference_time || Time.now
    end

    # Calculate recency factor for a single timestamp.
    # Returns value between (1 - aging_factor) and 1.0
    #
    # @param created_at [String, Time, nil] ISO 8601 timestamp or Time object
    # @return [Float] Recency factor
    def calculate_recency_factor(created_at)
      return 1.0 if @aging_factor.zero?
      return 1.0 if created_at.nil?

      timestamp = parse_timestamp(created_at)
      return 1.0 if timestamp.nil?

      age_in_days = (@reference_time - timestamp) / SECONDS_PER_DAY
      age_in_days = [age_in_days, 0.0].max # Handle future timestamps

      exp_decay = Math.exp(-@lambda * age_in_days)
      1 - @aging_factor + (@aging_factor * exp_decay)
    end

    private

    def parse_timestamp(value)
      case value
      when Time
        value
      when String
        Time.parse(value)
      end
    rescue ArgumentError
      nil
    end
  end
end
```

**Step 4: Add require to lib/recollect.rb**

Add after line 7 (after other requires):
```ruby
require_relative "recollect/recency_ranker"
```

**Step 5: Run tests to verify they pass**

Run: `bundle exec ruby -Itest test/recollect/recency_ranker_test.rb`
Expected: All 8 tests PASS

**Step 6: Commit**

```bash
git add lib/recollect/recency_ranker.rb lib/recollect.rb test/recollect/recency_ranker_test.rb
git commit -m "feat: add RecencyRanker with exponential decay calculation"
```

---

## Task 3: Add apply() Method to RecencyRanker

**Files:**
- Modify: `lib/recollect/recency_ranker.rb`
- Modify: `test/recollect/recency_ranker_test.rb`

**Step 1: Write failing tests for apply()**

Add to `test/recollect/recency_ranker_test.rb`:

```ruby
def test_apply_returns_empty_for_empty_results
  ranker = Recollect::RecencyRanker.new(
    aging_factor: 1.0,
    half_life_days: 30,
    reference_time: @reference_time
  )

  results = ranker.apply([], score_field: "score")
  assert_empty results
end

def test_apply_skips_when_aging_factor_zero
  ranker = Recollect::RecencyRanker.new(
    aging_factor: 0.0,
    half_life_days: 30,
    reference_time: @reference_time
  )

  results = [
    {"id" => 1, "score" => 1.0, "created_at" => "2025-01-01T12:00:00Z"}
  ]

  ranked = ranker.apply(results, score_field: "score")

  assert_equal 1, ranked.first["id"]
  assert_in_delta 1.0, ranked.first["score"], 0.001
  refute ranked.first.key?("recency_factor")
end

def test_apply_adjusts_scores
  ranker = Recollect::RecencyRanker.new(
    aging_factor: 1.0,
    half_life_days: 14,
    reference_time: @reference_time
  )

  results = [
    {"id" => 1, "score" => 1.0, "created_at" => "2025-01-01T12:00:00Z"} # 14 days old
  ]

  ranked = ranker.apply(results, score_field: "score")

  # Score should be halved (1.0 * 0.5 = 0.5)
  assert_in_delta 0.5, ranked.first["score"], 0.01
  assert_in_delta 0.5, ranked.first["recency_factor"], 0.01
end

def test_apply_resorts_by_adjusted_score
  ranker = Recollect::RecencyRanker.new(
    aging_factor: 1.0,
    half_life_days: 7,
    reference_time: @reference_time
  )

  results = [
    {"id" => 1, "score" => 1.0, "created_at" => "2025-01-01T12:00:00Z"}, # Old, high score
    {"id" => 2, "score" => 0.4, "created_at" => "2025-01-15T12:00:00Z"}, # New, low score
    {"id" => 3, "score" => 0.6, "created_at" => "2025-01-08T12:00:00Z"}  # Medium
  ]

  ranked = ranker.apply(results, score_field: "score")

  # Order by adjusted score: new beats old despite lower original
  assert_equal [3, 2, 1], ranked.map { |r| r["id"] }
end

def test_apply_preserves_original_data
  ranker = Recollect::RecencyRanker.new(
    aging_factor: 1.0,
    half_life_days: 30,
    reference_time: @reference_time
  )

  results = [
    {"id" => 1, "score" => 1.0, "created_at" => "2025-01-10T12:00:00Z", "content" => "test"}
  ]

  ranked = ranker.apply(results, score_field: "score")

  assert_equal "test", ranked.first["content"]
  assert_equal 1, ranked.first["id"]
end
```

**Step 2: Run tests to verify they fail**

Run: `bundle exec ruby -Itest test/recollect/recency_ranker_test.rb -n /apply/`
Expected: FAIL with "undefined method `apply'"

**Step 3: Implement apply() method**

Add to `lib/recollect/recency_ranker.rb` after `calculate_recency_factor`:

```ruby
# Apply recency ranking to results.
# Results must have a score field and created_at timestamp.
#
# @param results [Array<Hash>] Search results with score and created_at
# @param score_field [String] Name of the score field to modify
# @return [Array<Hash>] Re-sorted results with adjusted scores
def apply(results, score_field:)
  return results if @aging_factor.zero? || results.empty?

  results.map do |result|
    recency_factor = calculate_recency_factor(result["created_at"])
    original_score = result[score_field] || 0.0
    adjusted_score = original_score * recency_factor

    result.merge(
      score_field => adjusted_score,
      "recency_factor" => recency_factor
    )
  end.sort_by { |r| -(r[score_field] || 0.0) }
end
```

**Step 4: Run tests to verify they pass**

Run: `bundle exec ruby -Itest test/recollect/recency_ranker_test.rb`
Expected: All 13 tests PASS

**Step 5: Commit**

```bash
git add lib/recollect/recency_ranker.rb test/recollect/recency_ranker_test.rb
git commit -m "feat(recency): add apply() method to re-rank results"
```

---

## Task 4: Integrate RecencyRanker with HybridSearchRanker

**Files:**
- Modify: `lib/recollect/hybrid_search_ranker.rb`
- Modify: `test/recollect/hybrid_search_ranker_test.rb`

**Step 1: Write failing test for recency integration**

Add to `test/recollect/hybrid_search_ranker_test.rb`:

```ruby
def test_merge_with_recency_ranker
  reference_time = Time.parse("2025-01-15T12:00:00Z")
  recency_ranker = Recollect::RecencyRanker.new(
    aging_factor: 1.0,
    half_life_days: 7,
    reference_time: reference_time
  )

  fts_results = [
    {"id" => 1, "content" => "old but relevant", "rank" => -10.0,
     "created_at" => "2025-01-01T12:00:00Z"}, # 14 days old = 2 half-lives
    {"id" => 2, "content" => "recent but less relevant", "rank" => -5.0,
     "created_at" => "2025-01-14T12:00:00Z"}  # 1 day old
  ]
  vec_results = []

  results = Recollect::HybridSearchRanker.merge(
    fts_results, vec_results,
    limit: 10,
    recency_ranker: recency_ranker
  )

  # Recent item should be boosted higher despite lower initial score
  assert_equal 2, results.first["id"]
  assert results.first.key?("recency_factor")
end

def test_merge_without_recency_ranker_unchanged
  fts_results = [
    {"id" => 1, "content" => "relevant", "rank" => -10.0,
     "created_at" => "2025-01-01T12:00:00Z"},
    {"id" => 2, "content" => "less relevant", "rank" => -5.0,
     "created_at" => "2025-01-14T12:00:00Z"}
  ]
  vec_results = []

  results = Recollect::HybridSearchRanker.merge(
    fts_results, vec_results,
    limit: 10
  )

  # Without recency, order by original relevance
  assert_equal 1, results.first["id"]
  refute results.first.key?("recency_factor")
end
```

**Step 2: Run tests to verify they fail**

Run: `bundle exec ruby -Itest test/recollect/hybrid_search_ranker_test.rb -n /recency/`
Expected: FAIL with "unknown keyword: :recency_ranker"

**Step 3: Modify HybridSearchRanker to accept recency_ranker**

In `lib/recollect/hybrid_search_ranker.rb`, update the `merge` method:

```ruby
def self.merge(fts_results, vec_results, limit:, recency_ranker: nil)
  scores = {}
  score_fts_results(fts_results, scores)
  score_vector_results(vec_results, scores)
  results = combine_and_sort(scores, recency_ranker ? limit * 2 : limit)

  if recency_ranker
    results = recency_ranker.apply(results, score_field: "combined_score")
  end

  results.take(limit)
end
```

**Step 4: Run tests to verify they pass**

Run: `bundle exec ruby -Itest test/recollect/hybrid_search_ranker_test.rb`
Expected: All tests PASS

**Step 5: Run full test suite**

Run: `bundle exec rake test`
Expected: All tests PASS

**Step 6: Commit**

```bash
git add lib/recollect/hybrid_search_ranker.rb test/recollect/hybrid_search_ranker_test.rb
git commit -m "feat(hybrid): add optional recency_ranker to merge()"
```

---

## Task 5: Wire Up Recency in DatabaseManager

**Files:**
- Modify: `lib/recollect/database_manager.rb`
- Modify: `test/recollect/database_manager_test.rb`

**Step 1: Write failing integration test**

Add to `test/recollect/database_manager_test.rb`:

```ruby
def test_hybrid_search_applies_recency_when_enabled
  # Skip if vectors not available
  skip "Vectors not available" unless Recollect.config.vectors_available?

  ENV["RECOLLECT_RECENCY_AGING_FACTOR"] = "1.0"
  ENV["RECOLLECT_RECENCY_HALF_LIFE_DAYS"] = "7"

  config = Recollect::Config.new
  manager = Recollect::DatabaseManager.new(config)

  db = manager.get_database("recency-test")
  db.store(content: "old memory about Ruby programming")
  id_new = db.store(content: "new memory about Ruby programming")

  # Backdate the first memory
  db.instance_variable_get(:@db).execute(
    "UPDATE memories SET created_at = ? WHERE id = 1",
    ["2024-01-01T00:00:00Z"]
  )

  criteria = Recollect::SearchCriteria.new(query: "Ruby", project: "recency-test")
  results = manager.hybrid_search(criteria)

  # New memory should rank higher due to recency
  assert_equal id_new, results.first["id"]
  assert results.first.key?("recency_factor")
ensure
  manager&.close_all
  ENV.delete("RECOLLECT_RECENCY_AGING_FACTOR")
  ENV.delete("RECOLLECT_RECENCY_HALF_LIFE_DAYS")
end

def test_search_all_applies_recency_when_enabled
  ENV["RECOLLECT_RECENCY_AGING_FACTOR"] = "1.0"
  ENV["RECOLLECT_RECENCY_HALF_LIFE_DAYS"] = "7"

  config = Recollect::Config.new
  manager = Recollect::DatabaseManager.new(config)

  db = manager.get_database("recency-fts-test")
  db.store(content: "old memory about Python coding")
  id_new = db.store(content: "new memory about Python coding")

  # Backdate the first memory
  db.instance_variable_get(:@db).execute(
    "UPDATE memories SET created_at = ? WHERE id = 1",
    ["2024-01-01T00:00:00Z"]
  )

  criteria = Recollect::SearchCriteria.new(query: "Python", project: "recency-fts-test")
  results = manager.search_all(criteria)

  # New memory should rank higher due to recency
  assert_equal id_new, results.first["id"]
  assert results.first.key?("recency_factor")
ensure
  manager&.close_all
  ENV.delete("RECOLLECT_RECENCY_AGING_FACTOR")
  ENV.delete("RECOLLECT_RECENCY_HALF_LIFE_DAYS")
end

def test_search_all_no_recency_when_disabled
  db = @manager.get_database("no-recency-test")
  db.store(content: "memory about patterns")

  criteria = Recollect::SearchCriteria.new(query: "patterns", project: "no-recency-test")
  results = @manager.search_all(criteria)

  # Should NOT have recency_factor since it's disabled by default
  refute results.first&.key?("recency_factor")
end
```

**Step 2: Run tests to verify they fail**

Run: `bundle exec ruby -Itest test/recollect/database_manager_test.rb -n /recency/`
Expected: FAIL (recency not being applied)

**Step 3: Implement recency wiring in DatabaseManager**

In `lib/recollect/database_manager.rb`:

Add private helper methods at the end of the class (before the closing `end`):

```ruby
def recency_enabled?
  @config.recency_enabled?
end

def build_recency_ranker
  RecencyRanker.new(
    aging_factor: @config.recency_aging_factor,
    half_life_days: @config.recency_half_life_days
  )
end

def apply_recency_ranking(results, score_field:)
  return results unless recency_enabled?

  build_recency_ranker.apply(results, score_field: score_field)
end
```

Modify `search_all` method (around line 41):

```ruby
def search_all(criteria)
  results = if criteria.project?
    search_project(criteria)
  else
    search_all_projects(criteria)
  end

  # Get more results when recency enabled for re-ranking
  effective_limit = recency_enabled? ? criteria.limit * 2 : criteria.limit
  sorted = results.sort_by { |m| m["rank"] || 0 }.take(effective_limit)

  # Apply recency ranking if enabled
  if recency_enabled?
    sorted = apply_recency_ranking(sorted, score_field: "rank")
  end

  sorted.take(criteria.limit)
end
```

Modify `hybrid_search` method (around line 63):

```ruby
def hybrid_search(criteria)
  # If vectors not available, fall back to FTS5 only
  return search_all(criteria) unless @config.vectors_available? && vectors_ready?

  # Get query embedding
  embed_text = criteria.query_string
  embedding = embedding_client.embed(embed_text)

  # Collect results from both methods using expanded limit
  expand_factor = recency_enabled? ? 3 : 2
  expanded_criteria = SearchCriteria.new(
    query: criteria.query,
    project: criteria.project,
    memory_type: criteria.memory_type,
    limit: criteria.limit * expand_factor,
    created_after: criteria.created_after,
    created_before: criteria.created_before
  )
  fts_results = search_all(expanded_criteria)
  vec_results = vector_search_all(embedding, expanded_criteria)

  # Merge and rank with optional recency
  HybridSearchRanker.merge(
    fts_results,
    vec_results,
    limit: criteria.limit,
    recency_ranker: recency_enabled? ? build_recency_ranker : nil
  )
end
```

**Step 4: Run tests to verify they pass**

Run: `bundle exec ruby -Itest test/recollect/database_manager_test.rb -n /recency/`
Expected: All 3 tests PASS

**Step 5: Run full test suite**

Run: `bundle exec rake test`
Expected: All tests PASS

**Step 6: Commit**

```bash
git add lib/recollect/database_manager.rb test/recollect/database_manager_test.rb
git commit -m "feat: wire recency ranking into search paths"
```

---

## Task 6: Update Documentation

**Files:**
- Modify: `CLAUDE.md`

**Step 1: Add env vars to table**

In `CLAUDE.md`, add to the Environment Variables table:

```markdown
| `RECOLLECT_RECENCY_AGING_FACTOR` | `0.0` | Recency ranking strength (0.0-1.0, 0=disabled) |
| `RECOLLECT_RECENCY_HALF_LIFE_DAYS` | `30.0` | Days until memory relevance decays to 50% |
```

**Step 2: Run rubocop**

Run: `bundle exec rubocop`
Expected: No offenses

**Step 3: Run full test suite with coverage**

Run: `bundle exec rake coverage`
Expected: All tests pass, coverage not degraded

**Step 4: Commit**

```bash
git add CLAUDE.md
git commit -m "docs: add recency ranking configuration to CLAUDE.md"
```

---

## Task 7: Final Verification

**Step 1: Run full test suite**

Run: `bundle exec rake test`
Expected: All tests PASS

**Step 2: Test manually**

```bash
# Start server with recency enabled
RECOLLECT_RECENCY_AGING_FACTOR=0.5 RECOLLECT_RECENCY_HALF_LIFE_DAYS=14 ./bin/server

# In another terminal, store some memories and search
./bin/recollect store "old memory" -p test
# Wait or backdate via sqlite
./bin/recollect store "new memory" -p test
./bin/recollect search "memory" -p test
# Verify newer memory ranks higher
```

**Step 3: Final commit if any fixups needed**

```bash
git status
# If clean, done. Otherwise fix and commit.
```

---

## Summary

| Task | Description | Tests |
|------|-------------|-------|
| 1 | Config options | 8 tests |
| 2 | RecencyRanker core | 8 tests |
| 3 | apply() method | 5 tests |
| 4 | HybridSearchRanker integration | 2 tests |
| 5 | DatabaseManager wiring | 3 tests |
| 6 | Documentation | - |
| 7 | Final verification | - |

**Total new tests:** ~26
