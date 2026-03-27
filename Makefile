include env

all: deb

deb: target/debian/wp360-codesys-ftp_0.1.1-1_arm64.deb

target/debian/wp360-codesys-ftp_0.1.1-1_arm64.deb: src/main.rs src/ftp.rs
	cargo deb --target aarch64-unknown-linux-gnu

clean:
	-rm -r target

.PHONY: all clean deb
