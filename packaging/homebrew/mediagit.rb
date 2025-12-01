# MediaGit Homebrew Formula
# This file should be updated automatically by the release workflow

class Mediagit < Formula
  desc "Git-based version control system optimized for large media files"
  homepage "https://mediagit.dev"
  version "0.1.0"
  license "AGPL-3.0"

  on_macos do
    if Hardware::CPU.intel?
      url "https://github.com/yourusername/mediagit-core/releases/download/v#{version}/mediagit-#{version}-x86_64-macos.tar.gz"
      sha256 "PLACEHOLDER_INTEL_SHA256"
    elsif Hardware::CPU.arm?
      url "https://github.com/yourusername/mediagit-core/releases/download/v#{version}/mediagit-#{version}-aarch64-macos.tar.gz"
      sha256 "PLACEHOLDER_ARM_SHA256"
    end
  end

  on_linux do
    if Hardware::CPU.intel?
      url "https://github.com/yourusername/mediagit-core/releases/download/v#{version}/mediagit-#{version}-x86_64-linux.tar.gz"
      sha256 "PLACEHOLDER_LINUX_INTEL_SHA256"
    elsif Hardware::CPU.arm?
      url "https://github.com/yourusername/mediagit-core/releases/download/v#{version}/mediagit-#{version}-aarch64-linux.tar.gz"
      sha256 "PLACEHOLDER_LINUX_ARM_SHA256"
    end
  end

  def install
    bin.install "mediagit"
  end

  test do
    system "#{bin}/mediagit", "--version"
  end
end
