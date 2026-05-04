# frozen_string_literal: true

require "test_helper"

class Recollect::Sync::PeersTest < Recollect::TestCase
  def setup
    super
    @store = Recollect::Sync::Store.new(Recollect.config.sync_db_path)
    @peers = Recollect::Sync::Peers.new(@store)
  end

  def teardown
    @store.close
    super
  end

  def peer_attrs(id)
    {peer_id: id, display_name: "n-#{id}", public_key: ("\x00" * 32).b, endpoint: "http://#{id}:7326"}
  end

  def test_add_and_list
    @peers.add(**peer_attrs("p1"))
    @peers.add(**peer_attrs("p2"))
    list = @peers.list

    assert_equal 2, list.size
    assert_equal %w[p1 p2].sort, list.map { |p| p[:peer_id] }.sort
  end

  def test_block
    @peers.add(**peer_attrs("p1"))
    @peers.block("p1")
    p1 = @peers.find("p1")

    assert_equal "blocked", p1[:status]
  end

  def test_subscribe_and_subscriptions
    @peers.add(**peer_attrs("p1"))
    @peers.subscribe("p1", "global")
    @peers.subscribe("p1", "personal-finance")

    assert_equal %w[global personal-finance].sort, @peers.subscriptions("p1").sort
  end

  def test_unsubscribe
    @peers.add(**peer_attrs("p1"))
    @peers.subscribe("p1", "global")
    @peers.unsubscribe("p1", "global")

    assert_empty @peers.subscriptions("p1")
  end

  def test_default_subscription_on_add
    @peers.add(**peer_attrs("p1"), default_subscription: "global")

    assert_equal ["global"], @peers.subscriptions("p1")
  end
end
