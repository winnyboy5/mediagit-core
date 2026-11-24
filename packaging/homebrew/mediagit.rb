# MediaGit Homebrew formula TEMPLATE.
# The `homebrew` job in .github/workflows/release.yml fills in the @...@
# placeholders from the release's .sha256 files and pushes the result to
# https://github.com/winnyboy5/homebrew-tap (Formula/mediagit.rb).
# Do not edit version or checksums here by hand.

class Mediagit < Formula
  desc "Version control for large media files"
  homepage "https://mediagit.dev"
  version "@VERSION@"
  license "BUSL-1.1"

  on_macos do
    if Hardware::CPU.intel?
      url "https://github.com/winnyboy5/mediagit-core/releases/download/v#{version}/mediagit-#{version}-x86_64-macos.tar.gz"
      sha256 "@SHA_X86_64_MACOS@"
    elsif Hardware::CPU.arm?
      url "https://github.com/winnyboy5/mediagit-core/releases/download/v#{version}/mediagit-#{version}-aarch64-macos.tar.gz"
      sha256 "@SHA_AARCH64_MACOS@"
    end
  end

  on_linux do
    if Hardware::CPU.intel?
      url "https://github.com/winnyboy5/mediagit-core/releases/download/v#{version}/mediagit-#{version}-x86_64-linux.tar.gz"
      sha256 "@SHA_X86_64_LINUX@"
    elsif Hardware::CPU.arm?
      url "https://github.com/winnyboy5/mediagit-core/releases/download/v#{version}/mediagit-#{version}-aarch64-linux.tar.gz"
      sha256 "@SHA_AARCH64_LINUX@"
    end
  end

  def install
    bin.install "mediagit"
    bin.install "mediagit-server"
  end

  test do
    system "#{bin}/mediagit", "--version"
  end
end
