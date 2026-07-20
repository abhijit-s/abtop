class Abtop < Formula
  desc "AI agent monitor for your terminal"
  homepage "https://github.com/abhijit-s/abtop"
  version "0.6.2"
  if OS.mac?
    if Hardware::CPU.arm?
      url "https://github.com/abhijit-s/abtop/releases/download/v0.6.2/abtop-aarch64-apple-darwin.tar.xz"
      sha256 "313c49252dbd7003132bf47108801455ada0bd0e611af41df7ed8d76e081ac18"
    end
    if Hardware::CPU.intel?
      url "https://github.com/abhijit-s/abtop/releases/download/v0.6.2/abtop-x86_64-apple-darwin.tar.xz"
      sha256 "e81bc916b94051d9e509e59e64808779bba2e6897ffe85a7dacead0caa6c6a08"
    end
  end
  if OS.linux?
    if Hardware::CPU.arm?
      url "https://github.com/abhijit-s/abtop/releases/download/v0.6.2/abtop-aarch64-unknown-linux-gnu.tar.xz"
      sha256 "bcebde827824203d97b2ad0e2912a1d10c0d8394133678011dd2705039abac0f"
    end
    if Hardware::CPU.intel?
      url "https://github.com/abhijit-s/abtop/releases/download/v0.6.2/abtop-x86_64-unknown-linux-gnu.tar.xz"
      sha256 "c3d671f4e970e11b5f261bbc38d8d7e4e9d002fa48cb99d792c0480266a1f6f3"
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
