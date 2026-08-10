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
