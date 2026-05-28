hid_seize_reports: hid_seize_reports.c
	clang -framework IOKit -framework CoreFoundation -o $@ $<
	codesign --sign - --force $@

clean:
	rm -f hid_seize_reports

.PHONY: clean
