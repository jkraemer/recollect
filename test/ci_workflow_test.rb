# frozen_string_literal: true

require "test_helper"
require "yaml"

# CI has to keep proving the oldest Ruby the gemspec promises to support.
# Nothing else notices if the two drift apart.
class CIWorkflowTest < Minitest::Test
  ROOT = File.expand_path("..", __dir__)

  def test_ci_workflow_is_valid_yaml
    refute_nil workflow("ci.yml"), "ci.yml must parse as YAML"
  end

  def test_ci_tests_the_oldest_supported_ruby
    assert_includes ruby_matrix, supported_ruby_floor,
      "the CI matrix must cover the gemspec's minimum Ruby"
  end

  def test_nightly_guards_the_vector_stack_before_running_the_suite
    commands = nightly_steps.map { |step| step["run"].to_s }

    guard = commands.index { |run| run.include?("verify-vector-stack") }
    suite = commands.index { |run| run.include?("rake coverage") }

    refute_nil guard, "the nightly must assert the vector stack is live"
    refute_nil suite, "the nightly must run rake coverage"
    assert_operator guard, :<, suite, "the guard must run before the suite, not after"
  end

  # A guard that can be silently skipped or ignored is worse than no guard: it
  # makes the nightly green regardless of whether the vector stack works.
  def test_nightly_guard_step_cannot_be_neutered
    guard_step = nightly_steps.find { |step| step["run"].to_s.include?("verify-vector-stack") }

    refute_nil guard_step, "the nightly must have a vector-stack guard step"
    refute guard_step.key?("continue-on-error"),
      "the guard must not tolerate failure -- continue-on-error would make the nightly green " \
        "regardless of the vector stack"
    refute guard_step.key?("if"),
      "the guard must not be conditionally skippable -- an if: could make the nightly green " \
        "regardless of the vector stack"
  end

  def test_ci_triggers_on_push_to_master_and_pull_request
    triggers = workflow_triggers("ci.yml")

    assert triggers.key?("push"), "ci.yml must run on push"
    assert_includes triggers.dig("push", "branches") || [], "master", "ci.yml must run on push to master"
    assert triggers.key?("pull_request"), "ci.yml must run on pull_request"
  end

  # Without a schedule, deleting the cron (or it never being restored after
  # an edit) silently stops the nightly forever -- and the nightly is the
  # only thing enforcing minimum_coverage and the only thing executing
  # vector code.
  def test_nightly_has_a_schedule_trigger
    triggers = workflow_triggers("nightly.yml")

    assert triggers.key?("schedule"), "nightly.yml must have a cron schedule"
  end

  # Auto-cancelling by github.ref applies to master pushes too: two quick
  # pushes leave the first commit with no CI result. Only PR runs are safe
  # to cancel (each new push supersedes the old head).
  def test_ci_cancels_only_pull_request_runs
    cancel = workflow("ci.yml").fetch("concurrency").fetch("cancel-in-progress")

    refute_equal true, cancel, "master pushes must not cancel each other's runs"
    assert_includes cancel.to_s, "pull_request",
      "cancel-in-progress should be conditional on the run being a pull_request"
  end

  # Without a concurrency group, a workflow_dispatch can race the cron over
  # the same cache key.
  def test_nightly_serializes_concurrent_runs
    concurrency = workflow("nightly.yml")["concurrency"]

    refute_nil concurrency, "nightly.yml must declare a concurrency group"
    refute_equal true, concurrency["cancel-in-progress"],
      "a dispatched run must queue behind the cron, not kill it mid-suite"
  end

  # A hung run without a timeout burns the 360-minute default.
  def test_ci_jobs_have_timeouts
    workflow("ci.yml").fetch("jobs").each do |name, job|
      assert job.key?("timeout-minutes"), "ci.yml job #{name.inspect} must set timeout-minutes"
    end
  end

  # A fixed cache key with no restore-keys is an exact hit forever once
  # saved: a partial entry would be permanently sticky, recoverable only by
  # deleting the cache or bumping the key string.
  def test_nightly_model_cache_can_recover_from_a_bad_entry
    cache_step = nightly_steps.find { |step| step.dig("with", "path").to_s.include?("huggingface") }

    refute_nil cache_step, "the nightly must cache the embedding model"
    assert cache_step["with"].key?("restore-keys"),
      "the model cache needs restore-keys so a fresh key can still restore prior entries"
  end

  # sqlite-vec's one home is requirements.txt; an inline pip argument in the
  # workflow is where version drift hides.
  def test_python_vector_stack_is_declared_in_requirements
    requirements = File.read(File.join(ROOT, "requirements.txt"))

    assert_match(/^sqlite-vec/, requirements, "requirements.txt must declare sqlite-vec")

    install = nightly_steps.map { |step| step["run"].to_s }.find { |run| run.include?("requirements.txt") }

    refute_nil install, "the nightly must install from requirements.txt"
    refute_match(/sqlite-vec/, install,
      "sqlite-vec belongs in requirements.txt, not inline in the workflow")
  end

  private

  def workflow(name)
    path = File.join(ROOT, ".github", "workflows", name)

    assert_path_exists path
    YAML.safe_load_file(path)
  end

  def nightly_steps
    workflow("nightly.yml").fetch("jobs").fetch("full-suite").fetch("steps")
  end

  # Under Psych (YAML 1.1), a bare `on:` key parses as the boolean `true`,
  # not the string "on" -- fetch accordingly, or this silently returns
  # nothing once someone "fixes" it back to "on".
  def workflow_triggers(name)
    workflow(name).fetch(true)
  end

  def ruby_matrix
    workflow("ci.yml").fetch("jobs").fetch("test").fetch("strategy").fetch("matrix").fetch("ruby")
  end

  # "3.4.0" from the gemspec becomes the "3.4" that setup-ruby wants.
  def supported_ruby_floor
    spec = Gem::Specification.load(File.join(ROOT, "recollect.gemspec"))
    spec.required_ruby_version.requirements.first.last.to_s.split(".").first(2).join(".")
  end
end
