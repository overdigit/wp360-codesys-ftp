include env

all: deb

deb: src/main.rs src/ftp.rs
	cargo deb --target aarch64-unknown-linux-gnu

clean:
	-rm -r target

.PHONY: all clean deb
