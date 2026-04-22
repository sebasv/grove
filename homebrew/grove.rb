# This file belongs in the tap repo (sebasv/homebrew-grove) at Formula/grove.rb
# The release workflow keeps it up to date automatically.
class Grove < Formula
  desc "A TUI for cultivating git repos, worktrees, and the work inside them"
  homepage "https://github.com/sebasv/grove"
  version "0.1.0"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/sebasv/grove/releases/download/v0.1.0/grove-aarch64-apple-darwin.tar.gz"
      sha256 "0000000000000000000000000000000000000000000000000000000000000000"
    end

    on_intel do
      url "https://github.com/sebasv/grove/releases/download/v0.1.0/grove-x86_64-apple-darwin.tar.gz"
      sha256 "0000000000000000000000000000000000000000000000000000000000000000"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/sebasv/grove/releases/download/v0.1.0/grove-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "0000000000000000000000000000000000000000000000000000000000000000"
    end

    on_intel do
      url "https://github.com/sebasv/grove/releases/download/v0.1.0/grove-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "0000000000000000000000000000000000000000000000000000000000000000"
    end
  end

  def install
    bin.install "grove"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/grove --version")
  end
end
