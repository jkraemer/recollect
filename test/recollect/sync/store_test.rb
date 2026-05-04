# frozen_string_literal: true

require "test_helper"

class Recollect::Sync::StoreTest < Recollect::TestCase
  def setup
    super
    @path = Recollect.config.data_dir.join("sync.db")
    @store = Recollect::Sync::Store.new(@path)
  end

  def teardown
    @store.close
    super
  end

  def test_creates_all_tables
    raw = @store.instance_variable_get(:@db)
    tables = raw.execute("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name").map { |r| r["name"] }

    %w[local_identity known_peers peer_watermarks pairing_codes peer_db_subscriptions].each do |t|
      assert_includes tables, t
    end
  end

  def test_file_mode_is_0600
    File.chmod(0o644, @path) # set wrong mode first
    @store.close
    @store = Recollect::Sync::Store.new(@path)
    mode = File.stat(@path).mode & 0o777

    assert_equal 0o600, mode
  end

  def test_idempotent_open
    @store.close
    again = Recollect::Sync::Store.new(@path)

    refute_nil again.instance_variable_get(:@db)
    again.close
  end
end
