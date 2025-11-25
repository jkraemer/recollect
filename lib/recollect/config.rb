# frozen_string_literal: true

require 'pathname'
require 'json'

module Recollect
  class Config
    attr_accessor :data_dir, :host, :port, :max_results

    def initialize
      @data_dir = Pathname.new(ENV.fetch('RECOLLECT_DATA_DIR',
        File.join(Dir.home, '.recollect')))
      @host = ENV.fetch('RECOLLECT_HOST', '127.0.0.1')
      @port = ENV.fetch('RECOLLECT_PORT', '8080').to_i
      @max_results = 100

      ensure_directories!
    end

    def global_db_path
      data_dir.join('global.db')
    end

    def projects_dir
      data_dir.join('projects')
    end

    def project_db_path(project_name)
      projects_dir.join("#{sanitize_name(project_name)}.db")
    end

    def detect_project(cwd = Dir.pwd)
      path = Pathname.new(cwd)

      # Check for .git
      if (path / '.git').exist?
        return git_remote_name(path) || path.basename.to_s
      end

      # Check for package.json
      if (path / 'package.json').exist?
        data = JSON.parse((path / 'package.json').read)
        return data['name'] if data['name']
      end

      # Check for *.gemspec
      gemspec = Dir.glob(path / '*.gemspec').first
      return File.basename(gemspec, '.gemspec') if gemspec

      # Fallback to directory name (unless generic)
      name = path.basename.to_s
      return nil if %w[home Documents Desktop Downloads src code].include?(name)

      name
    end

    private

    def ensure_directories!
      data_dir.mkpath
      projects_dir.mkpath
    end

    def sanitize_name(name)
      name.to_s.gsub(/[^a-zA-Z0-9_]/, '_').downcase
    end

    def git_remote_name(path)
      output = `git -C "#{path}" config --get remote.origin.url 2>/dev/null`.strip
      return nil if output.empty?

      # Extract repo name from URL
      output.split('/').last&.sub(/\.git$/, '')
    rescue StandardError
      nil
    end
  end
end
