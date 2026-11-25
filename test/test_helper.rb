# frozen_string_literal: true

$LOAD_PATH.unshift File.expand_path('../lib', __dir__)

require 'maxmind/db/rust'
require 'minitest/autorun'
require 'open3'
require 'rbconfig'

# Runs Ruby code in a child process, for code that can crash or hang the VM.
module ChildProcessHelper
  CHILD_TIMEOUT = 30

  def run_child(script)
    Open3.popen3(RbConfig.ruby, '-e', script) do |stdin, stdout, stderr, wait_thr|
      stdin.close
      # Read while waiting so a child that writes a lot cannot block on a
      # full pipe and look like a deadlock.
      readers = [stdout, stderr].map do |io|
        Thread.new { io.read }.tap { |thread| thread.report_on_exception = false }
      end

      unless wait_thr.join(CHILD_TIMEOUT)
        # SIGKILL because a deadlocked process may not exit on SIGTERM.
        Process.kill(:KILL, wait_thr.pid)
        wait_thr.join

        flunk "child process did not finish within #{CHILD_TIMEOUT} seconds"
      end

      [wait_thr.value, *readers.map(&:value)]
    end
  end
end
