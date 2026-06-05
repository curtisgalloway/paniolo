all: hid_seize_reports hid_capture_usb

hid_seize_reports: hid_seize_reports.c
	clang -framework IOKit -framework CoreFoundation -o $@ $<
	codesign --sign - --force $@

hid_capture_usb: hid_capture_usb.m
	clang -fobjc-arc -framework Foundation -framework IOKit -framework IOUSBHost -o $@ $<
	codesign --sign - --force $@

clean:
	rm -f hid_seize_reports hid_capture_usb

.PHONY: all clean
