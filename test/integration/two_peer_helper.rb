# frozen_string_literal: true

require "test_helper"
require "faraday"
require "faraday/rack"
require "tmpdir"
require "pathname"

# Harness for running two HTTPServer instances in one Ruby process.
#
# Each peer gets its own data_dir, Config instance, and dynamic HTTPServer
# subclass so that class-level singletons (sync_store, local_identity,
# db_manager, etc.) stay isolated between peers. Cross-peer HTTP goes through
# Faraday's :rack adapter pointing at the other peer's Sinatra subclass.
module TwoPeerHelper
  Peer = Struct.new(:label, :data_dir, :config, :app_class, keyword_init: true) do
    # Swaps Recollect.config to this peer's config for the duration of the block.
    # Required because db_manager, sync_store, etc. read Recollect.config when
    # lazily initialized.
    def with_config
      previous = Recollect.instance_variable_get(:@config)
      Recollect.instance_variable_set(:@config, config)
      yield
    ensure
      Recollect.instance_variable_set(:@config, previous)
    end

    def identity
      with_config { app_class.local_identity }
    end

    def sync_store
      with_config { app_class.sync_store }
    end

    def db_manager
      with_config { app_class.db_manager }
    end

    def store(content:, project: nil)
      with_config { app_class.memories_service.create(content: content, project: project) }
    end

    def delete(id, project: nil)
      with_config { app_class.memories_service.delete(id, project: project) }
    end

    def list_global
      with_config { app_class.memories_service.list(project: nil) }
    end
  end

  # Build an isolated peer. Each peer:
  #   - lives in its own tmp data_dir
  #   - has its own Config (data_dir overridden)
  #   - has its own HTTPServer subclass with a fresh @init_mutex so the
  #     class-level singleton ivars (e.g. @db_manager) don't collide with
  #     the parent class or the sibling peer.
  def make_peer(label)
    dir = Pathname.new(Dir.mktmpdir("recollect-#{label}-"))

    # Construct Config under an ENV override so data_dir picks up the tmp path.
    previous_data_dir = ENV["RECOLLECT_DATA_DIR"]
    ENV["RECOLLECT_DATA_DIR"] = dir.to_s
    cfg = Recollect::Config.new
    ENV["RECOLLECT_DATA_DIR"] = previous_data_dir

    klass = Class.new(Recollect::HTTPServer)
    klass.instance_variable_set(:@init_mutex, Mutex.new)

    Peer.new(label: label, data_dir: dir, config: cfg, app_class: klass)
  end

  # Returns a lambda suitable for Sync::Engine's :client_factory. The client
  # signs as `from` and routes its HTTP calls through `to`'s Rack stack.
  def cross_peer_client_factory(from:, to:)
    from_identity = from.identity
    ->(_peer_descriptor) {
      Recollect::Sync::Client.new(
        peer_id: from_identity.peer_id,
        private_key: from_identity.private_key,
        endpoint: "http://#{to.label}.test",
        adapter: [:rack, to.app_class]
      )
    }
  end

  # Mutually register the two peers as trusted, both subscribed to "global".
  def pair!(from:, to:)
    from_id = from.identity
    to_id = to.identity

    from.with_config do
      Recollect::Sync::Peers.new(from.app_class.sync_store).add(
        peer_id: to_id.peer_id, display_name: to.label.to_s,
        public_key: to_id.public_key, endpoint: "http://#{to.label}.test",
        default_subscription: "global"
      )
    end
    to.with_config do
      Recollect::Sync::Peers.new(to.app_class.sync_store).add(
        peer_id: from_id.peer_id, display_name: from.label.to_s,
        public_key: from_id.public_key, endpoint: "http://#{from.label}.test",
        default_subscription: "global"
      )
    end
  end

  def teardown_peer(peer)
    peer.with_config { peer.app_class.reset_db_manager! }
    FileUtils.remove_entry(peer.data_dir.to_s)
  end
end
