prefix = /usr

all: src/wp360-codesys-bridge #target/release/wp360-codesys-bridge-rs

target/release/wp360-codesys-bridge-rs: src/main.rs src/ftp.rs
	cargo build --release

install: all
	install -d $(DESTDIR)$(prefix)/bin
	install -d $(DESTDIR)$(prefix)/lib/wp360-codesys-bridge
	install src/wp360-codesys-bridge $(DESTDIR)$(prefix)/bin/
#	install target/release/wp360-codesys-bridge-rs $(DESTDIR)$(prefix)/lib/wp360-codesys-bridge/wp360-codesys-bridge

clean:
	-rm -r target

.PHONY: all install clean
