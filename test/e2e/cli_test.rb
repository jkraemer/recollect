# frozen_string_literal: true

require "test_helper"
require "net/http"
require "open3"

# Drives the real bin/recollect binary against a real server process,
# mirroring how the sync runbook exercises the CLI.
class CLIEndToEndTest < Recollect::TestCase
  ROOT = File.expand_path("../..", __dir__)
  HEALTH_TIMEOUT = 15

  def setup
    super
    @data_dir = File.join(ROOT, "test/tmp/e2e_data")
    FileUtils.rm_rf(@data_dir)
    @port = find_free_port
    @server_log = File.join(ROOT, "test/tmp/e2e_server.log")
    @server_pid = Process.spawn(
      {"RECOLLECT_DATA_DIR" => @data_dir, "RECOLLECT_PORT" => @port.to_s, "RECOLLECT_SYNC_DISABLE" => "1"},
      File.join(ROOT, "bin/server"),
      :chdir => ROOT, [:out, :err] => @server_log
    )
    wait_for_health
  end

  def teardown
    if @server_pid
      Process.kill("TERM", @server_pid)
      Process.wait(@server_pid)
    end
    FileUtils.rm_rf(@data_dir)
    FileUtils.rm_f(@server_log)
    super
  end

  def test_delete_removes_a_memory
    out, = run_cli("store", "doomed memory")
    id = out[/Stored memory #(\d+)/, 1]

    refute_nil id, "store should report an id, got: #{out}"

    out, status = run_cli("delete", id)

    assert_predicate status, :success?
    assert_match(/Deleted memory ##{id}/, out)

    out, = run_cli("list")

    refute_match(/doomed memory/, out)
  end

  def test_delete_scoped_to_project
    out, = run_cli("store", "project doomed", "-p", "someproj")
    id = out[/Stored memory #(\d+)/, 1]

    run_cli("delete", id, "-p", "someproj")

    out, = run_cli("list", "-p", "someproj")

    refute_match(/project doomed/, out)
  end

  def test_show_prints_full_content
    # Tables truncate content at 50 chars; show must print all of it.
    content = "a memory long enough that the table view would certainly truncate it somewhere"
    out, = run_cli("store", content)
    id = out[/Stored memory #(\d+)/, 1]

    out, status = run_cli("show", id)

    assert_predicate status, :success?
    assert_includes out, content
  end

  def test_show_missing_memory_fails
    out, status = run_cli("show", "999")

    refute_predicate status, :success?
    assert_match(/Error/, out)
  end

  def test_unknown_command_exits_nonzero_without_deprecation_noise
    out, status = run_cli("bogus")

    refute_predicate status, :success?
    refute_match(/Deprecation warning/, out)
  end

  def test_delete_reports_error_for_missing_memory
    out, status = run_cli("delete", "999")

    assert_match(/Error/, out)
    refute_predicate status, :success?
  end

  private

  def find_free_port
    server = TCPServer.new("127.0.0.1", 0)
    port = server.addr[1]
    server.close
    port
  end

  def wait_for_health
    deadline = Time.now + HEALTH_TIMEOUT
    loop do
      response = Net::HTTP.get_response(URI("http://127.0.0.1:#{@port}/health"))
      return if response.is_a?(Net::HTTPSuccess)
    rescue Errno::ECONNREFUSED, Errno::ECONNRESET
      raise "server did not become healthy: #{File.read(@server_log)}" if Time.now > deadline

      sleep 0.05
    end
  end

  def run_cli(*args)
    Open3.capture2e(
      {"RECOLLECT_URL" => "http://127.0.0.1:#{@port}"},
      File.join(ROOT, "bin/recollect"), *args, chdir: ROOT
    )
  end
end
