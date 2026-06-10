class Abtop < Formula
  desc "AI agent monitor for your terminal"
  homepage "https://github.com/abhijit-s/abtop"
  version "0.5.0"
  if OS.mac?
    if Hardware::CPU.arm?
      url "https://github.com/abhijit-s/abtop/releases/download/v0.5.0/abtop-aarch64-apple-darwin.tar.xz"
      sha256 "a5fefc07798bdf31a77844266fd24f30bed3e2696752e5542e16f18d50d62393"
    end
    if Hardware::CPU.intel?
      url "https://github.com/abhijit-s/abtop/releases/download/v0.5.0/abtop-x86_64-apple-darwin.tar.xz"
      sha256 "8997971cc52bc09529b015324bc621cba6024e6061532b2399d8da007d8a669e"
    end
  end
  if OS.linux?
    if Hardware::CPU.arm?
      url "https://github.com/abhijit-s/abtop/releases/download/v0.5.0/abtop-aarch64-unknown-linux-gnu.tar.xz"
      sha256 "27dc26dad3a2417603d7efa60c685a961bb2cb473854e872b89cb29f79e45187"
    end
    if Hardware::CPU.intel?
      url "https://github.com/abhijit-s/abtop/releases/download/v0.5.0/abtop-x86_64-unknown-linux-gnu.tar.xz"
      sha256 "b3f3748ca985aad910d8b72a0a21692eda76cc021952642bf4f5ea7fd46abee1"
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
