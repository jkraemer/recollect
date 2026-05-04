# frozen_string_literal: true

require "test_helper"

class Recollect::Sync::EndpointTest < Minitest::Test
  def setup
    @port = 7326
  end

  def test_env_var_wins
    ENV["RECOLLECT_PUBLIC_URL"] = "http://override.example:9000"

    assert_equal "http://override.example:9000", Recollect::Sync::Endpoint.discover(port: @port)
  ensure
    ENV.delete("RECOLLECT_PUBLIC_URL")
  end

  def test_falls_back_to_tailscale_when_env_unset
    Recollect::Sync::Endpoint.stub(:tailscale_dns_name, "laptop.tailnet.ts.net") do
      assert_equal "http://laptop.tailnet.ts.net:#{@port}", Recollect::Sync::Endpoint.discover(port: @port)
    end
  end

  def test_falls_back_to_lan_ip_when_no_tailscale
    Recollect::Sync::Endpoint.stub(:tailscale_dns_name, nil) do
      Recollect::Sync::Endpoint.stub(:lan_ipv4, "192.168.1.42") do
        assert_equal "http://192.168.1.42:#{@port}", Recollect::Sync::Endpoint.discover(port: @port)
      end
    end
  end

  def test_raises_when_nothing_resolves
    Recollect::Sync::Endpoint.stub(:tailscale_dns_name, nil) do
      Recollect::Sync::Endpoint.stub(:lan_ipv4, nil) do
        err = assert_raises(Recollect::Sync::Endpoint::DiscoveryError) do
          Recollect::Sync::Endpoint.discover(port: @port)
        end

        assert_match(/RECOLLECT_PUBLIC_URL/, err.message)
      end
    end
  end

  def test_raises_when_public_url_lacks_scheme
    ENV["RECOLLECT_PUBLIC_URL"] = "no-scheme.example:9000"
    err = assert_raises(Recollect::Sync::Endpoint::DiscoveryError) do
      Recollect::Sync::Endpoint.discover(port: @port)
    end

    assert_match(/scheme/, err.message)
  ensure
    ENV.delete("RECOLLECT_PUBLIC_URL")
  end

  def test_raises_on_invalid_public_url
    ENV["RECOLLECT_PUBLIC_URL"] = "ht!tp://bad url with spaces"
    err = assert_raises(Recollect::Sync::Endpoint::DiscoveryError) do
      Recollect::Sync::Endpoint.discover(port: @port)
    end

    assert_match(/RECOLLECT_PUBLIC_URL/, err.message)
  ensure
    ENV.delete("RECOLLECT_PUBLIC_URL")
  end
end
