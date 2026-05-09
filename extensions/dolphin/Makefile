DESTDIR ?=
PREFIX ?= $(HOME)/.local

# KDE service menu location (KF5/KF6 compatible).
SERVICE_MENU_DIR = $(PREFIX)/share/kservices5/ServiceMenus
# Alternative for KF6: $(PREFIX)/share/kio/servicemenus

DOLPHIN_EXT_DIR = $(PREFIX)/share/gdriver-dolphin
ICON_BASE = $(PREFIX)/share/icons/hicolor/scalable/emblems

EMBLEMS = emblem-gdriver-cloud.svg \
          emblem-gdriver-synced.svg \
          emblem-gdriver-syncing.svg \
          emblem-gdriver-error.svg

.PHONY: install uninstall icons desktop

install: icons desktop
	@mkdir -p $(DESTDIR)$(DOLPHIN_EXT_DIR)
	install -m 0755 gdriver_dolphin_ipc.py $(DESTDIR)$(DOLPHIN_EXT_DIR)/gdriver_dolphin_ipc.py
	install -m 0755 gdriver_dolphin_menu.py $(DESTDIR)$(DOLPHIN_EXT_DIR)/gdriver_dolphin_menu.py
	@echo "Installed Dolphin extension to $(DESTDIR)$(DOLPHIN_EXT_DIR)"

desktop:
	@mkdir -p $(DESTDIR)$(SERVICE_MENU_DIR)
	@# Substitute the install path into the .desktop file.
	sed 's|$${INSTALL_DIR}|$(DOLPHIN_EXT_DIR)|g' gdriver-dolphin.desktop \
		> $(DESTDIR)$(SERVICE_MENU_DIR)/gdriver-dolphin.desktop
	chmod 0644 $(DESTDIR)$(SERVICE_MENU_DIR)/gdriver-dolphin.desktop
	@echo "Installed service menu to $(DESTDIR)$(SERVICE_MENU_DIR)"

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
	@rm -f $(DESTDIR)$(DOLPHIN_EXT_DIR)/gdriver_dolphin_ipc.py
	@rm -f $(DESTDIR)$(DOLPHIN_EXT_DIR)/gdriver_dolphin_menu.py
	@rm -f $(DESTDIR)$(SERVICE_MENU_DIR)/gdriver-dolphin.desktop
	@for svg in $(EMBLEMS); do \
		rm -f $(DESTDIR)$(ICON_BASE)/$$svg; \
	done
	@if command -v gtk-update-icon-cache >/dev/null 2>&1; then \
		gtk-update-icon-cache -f -t $(PREFIX)/share/icons/hicolor; \
	fi
	@echo "Uninstalled Dolphin extension"
