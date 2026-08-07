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

  private

  def workflow(name)
    path = File.join(ROOT, ".github", "workflows", name)

    assert_path_exists path
    YAML.safe_load_file(path)
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
