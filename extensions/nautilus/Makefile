DESTDIR ?=
PREFIX ?= $(HOME)/.local

NAUTILUS_EXT_DIR = $(PREFIX)/share/nautilus-python/extensions
ICON_BASE = $(PREFIX)/share/icons/hicolor/scalable/emblems

EMBLEMS = emblem-gdriver-cloud.svg \
          emblem-gdriver-synced.svg \
          emblem-gdriver-syncing.svg \
          emblem-gdriver-error.svg

.PHONY: install uninstall icons

install: icons
	@mkdir -p $(DESTDIR)$(NAUTILUS_EXT_DIR)
	install -m 0644 gdriver_nautilus.py $(DESTDIR)$(NAUTILUS_EXT_DIR)/gdriver_nautilus.py
	install -m 0644 gdriver_ipc.py $(DESTDIR)$(NAUTILUS_EXT_DIR)/gdriver_ipc.py
	@echo "Installed Nautilus extension to $(DESTDIR)$(NAUTILUS_EXT_DIR)"

icons:
	@mkdir -p $(DESTDIR)$(ICON_BASE)
	@for svg in $(EMBLEMS); do \
		install -m 0644 icons/$$svg $(DESTDIR)$(ICON_BASE)/$$svg; \
	done
	@echo "Installed emblem icons to $(DESTDIR)$(ICON_BASE)"
	@if command -v gtk-update-icon-cache >/dev/null 2>&1; then \
		gtk-update-icon-cache -f -t $(PREFIX)/share/icons/hicolor; \
	fi

uninstall:
	@rm -f $(DESTDIR)$(NAUTILUS_EXT_DIR)/gdriver_nautilus.py
	@rm -f $(DESTDIR)$(NAUTILUS_EXT_DIR)/gdriver_ipc.py
	@for svg in $(EMBLEMS); do \
		rm -f $(DESTDIR)$(ICON_BASE)/$$svg; \
	done
	@if command -v gtk-update-icon-cache >/dev/null 2>&1; then \
		gtk-update-icon-cache -f -t $(PREFIX)/share/icons/hicolor; \
	fi
	@echo "Uninstalled Nautilus extension"
