# frozen_string_literal: true

require "json"
require "open3"
require "socket"

module Recollect
  module Sync
    module Endpoint
      class DiscoveryError < StandardError; end

      module_function

      def discover(port:)
        if (env = ENV["RECOLLECT_PUBLIC_URL"]) && !env.empty?
          return env
        end
        if (name = tailscale_dns_name)
          return "http://#{name}:#{port}"
        end
        if (ip = lan_ipv4)
          return "http://#{ip}:#{port}"
        end

        raise DiscoveryError, "Could not auto-detect a reachable URL. Set RECOLLECT_PUBLIC_URL or pass --endpoint."
      end

      def tailscale_dns_name
        out, status = Open3.capture2("tailscale", "status", "--json")
        return nil unless status.success?

        data = JSON.parse(out)
        self_node = data["Self"] or return nil
        dns = self_node["DNSName"]
        dns&.chomp(".") if dns && !dns.empty?
      rescue Errno::ENOENT, JSON::ParserError
        nil
      end

      def lan_ipv4
        Socket.ip_address_list.find do |a|
          a.ipv4? && !a.ipv4_loopback? && !a.ipv4_linklocal?
        end&.ip_address
      end
    end
  end
end
