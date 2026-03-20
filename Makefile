prefix = /usr

all: target/release/wp360-codesys-ftp

target/release/wp360-codesys-ftp: src/main.rs src/ftp.rs
	cargo build --release

install: all
	install -d $(DESTDIR)$(prefix)/lib/wp360-codesys-ftp
	install target/release/wp360-codesys-ftp $(DESTDIR)$(prefix)/lib/wp360-codesys-bridge/wp360-codesys-ftp

clean:
	-rm -r target

.PHONY: all install clean
