class Abtop < Formula
  desc "AI agent monitor for your terminal"
  homepage "https://github.com/abhijit-s/abtop"
  version "0.6.0"
  if OS.mac?
    if Hardware::CPU.arm?
      url "https://github.com/abhijit-s/abtop/releases/download/v0.6.0/abtop-aarch64-apple-darwin.tar.xz"
      sha256 "49b4f13b0206a9dc3317da74c9a3d83ba0f2ab949f041ee3f156ba358722c06f"
    end
    if Hardware::CPU.intel?
      url "https://github.com/abhijit-s/abtop/releases/download/v0.6.0/abtop-x86_64-apple-darwin.tar.xz"
      sha256 "7c96711ddd27826102a94068a32cd67a4bb40964951e0eaafb0dd0204cbe2b55"
    end
  end
  if OS.linux?
    if Hardware::CPU.arm?
      url "https://github.com/abhijit-s/abtop/releases/download/v0.6.0/abtop-aarch64-unknown-linux-gnu.tar.xz"
      sha256 "c9f9728c0b6b1ec510fdf4246f01dabfb16bc3f8e7d4686da3a8a053ef3b7079"
    end
    if Hardware::CPU.intel?
      url "https://github.com/abhijit-s/abtop/releases/download/v0.6.0/abtop-x86_64-unknown-linux-gnu.tar.xz"
      sha256 "a8711c5b0c755e32df472c714aa04ced9958943393931e45e8a929342ab4dc7b"
    end
  end
  license "MIT"

  BINARY_ALIASES = {
    "aarch64-apple-darwin": {},
    "aarch64-unknown-linux-gnu": {},
    "x86_64-apple-darwin": {},
    "x86_64-unknown-linux-gnu": {}
  }

  def target_triple
    cpu = Hardware::CPU.arm? ? "aarch64" : "x86_64"
    os = OS.mac? ? "apple-darwin" : "unknown-linux-gnu"

    "#{cpu}-#{os}"
  end

  def install_binary_aliases!
    BINARY_ALIASES[target_triple.to_sym].each do |source, dests|
      dests.each do |dest|
        bin.install_symlink bin/source.to_s => dest
      end
    end
  end

  def install
    if OS.mac? && Hardware::CPU.arm?
      bin.install "abtop"
    end
    if OS.mac? && Hardware::CPU.intel?
      bin.install "abtop"
    end
    if OS.linux? && Hardware::CPU.arm?
      bin.install "abtop"
    end
    if OS.linux? && Hardware::CPU.intel?
      bin.install "abtop"
    end

    install_binary_aliases!

    # Homebrew will automatically install these, so we don't need to do that
    doc_files = Dir["README.*", "readme.*", "LICENSE", "LICENSE.*", "CHANGELOG.*"]
    leftover_contents = Dir["*"] - doc_files

    # Install any leftover files in pkgshare; these are probably config or
    # sample files.
    pkgshare.install(*leftover_contents) unless leftover_contents.empty?
  end
end
