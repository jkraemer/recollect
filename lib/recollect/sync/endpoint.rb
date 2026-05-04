# frozen_string_literal: true

require "json"
require "open3"
require "socket"
require "uri"

module Recollect
  module Sync
    module Endpoint
      class DiscoveryError < StandardError; end

      module_function

      def discover(port:)
        if (env = ENV["RECOLLECT_PUBLIC_URL"]) && !env.empty?
          return validate_url!(env)
        end
        if (name = tailscale_dns_name)
          return "http://#{name}:#{port}"
        end
        if (ip = lan_ipv4)
          return "http://#{ip}:#{port}"
        end

        raise DiscoveryError, "Could not auto-detect a reachable URL. Set RECOLLECT_PUBLIC_URL or pass --endpoint."
      end

      def validate_url!(url)
        uri = URI.parse(url)
        unless %w[http https].include?(uri.scheme)
          raise DiscoveryError, "RECOLLECT_PUBLIC_URL must include a scheme (http:// or https://). Got: #{url.inspect}"
        end

        url
      rescue URI::InvalidURIError
        raise DiscoveryError, "RECOLLECT_PUBLIC_URL is not a valid URL: #{url.inspect}"
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
