class Abtop < Formula
  desc "AI agent monitor for your terminal"
  homepage "https://github.com/abhijit-s/abtop"
  version "0.5.1"
  if OS.mac?
    if Hardware::CPU.arm?
      url "https://github.com/abhijit-s/abtop/releases/download/v0.5.1/abtop-aarch64-apple-darwin.tar.xz"
      sha256 "7c83d11117e6046993519e7d0c0fbb7c50907d820ed66858648bce486c869ec0"
    end
    if Hardware::CPU.intel?
      url "https://github.com/abhijit-s/abtop/releases/download/v0.5.1/abtop-x86_64-apple-darwin.tar.xz"
      sha256 "49c473f42f9578b9ffe3235eeb1f2f0fbd681671865430529e7c94e2a1c5583a"
    end
  end
  if OS.linux?
    if Hardware::CPU.arm?
      url "https://github.com/abhijit-s/abtop/releases/download/v0.5.1/abtop-aarch64-unknown-linux-gnu.tar.xz"
      sha256 "1807825e496e64bf2e60071aaf2f909ce7ecac84c76c8d0e314156015790e012"
    end
    if Hardware::CPU.intel?
      url "https://github.com/abhijit-s/abtop/releases/download/v0.5.1/abtop-x86_64-unknown-linux-gnu.tar.xz"
      sha256 "ca36364685d182d8b349684559b77f96fc94d485ca7f1e58875717035f119b3d"
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
