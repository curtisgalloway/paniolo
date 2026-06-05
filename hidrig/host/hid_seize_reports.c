// hid_seize_reports.c
//
// Exclusively seize a single HID device on macOS and receive its RAW input
// reports — suitable for forwarding to a simulated HID device.
//
// Build:
//   clang -framework IOKit -framework CoreFoundation -o hid_seize_reports hid_seize_reports.c
//
// Run (Input Monitoring permission required):
//   ./hid_seize_reports
//
// Edit kVendorID / kProductID below to target your device.

#include <IOKit/hid/IOHIDManager.h>
#include <CoreFoundation/CoreFoundation.h>
#include <stdio.h>
#include <stdlib.h>
#include <signal.h>
#include <time.h>

// KB2040 running CircuitPython HID target firmware
static const long kVendorID  = 0x239A;
static const long kProductID = 0x8106;

static IOHIDManagerRef gManager = NULL;

static CFMutableDictionaryRef CreateMatchingDict(long vid, long pid) {
    CFMutableDictionaryRef d = CFDictionaryCreateMutable(
        kCFAllocatorDefault, 0,
        &kCFTypeDictionaryKeyCallBacks,
        &kCFTypeDictionaryValueCallBacks);
    if (!d) return NULL;
    if (vid) {
        CFNumberRef n = CFNumberCreate(kCFAllocatorDefault, kCFNumberLongType, &vid);
        CFDictionarySetValue(d, CFSTR(kIOHIDVendorIDKey), n);
        CFRelease(n);
    }
    if (pid) {
        CFNumberRef n = CFNumberCreate(kCFAllocatorDefault, kCFNumberLongType, &pid);
        CFDictionarySetValue(d, CFSTR(kIOHIDProductIDKey), n);
        CFRelease(n);
    }
    return d;
}

// Query an integer property from the device, with a fallback default.
static long GetDeviceLongProperty(IOHIDDeviceRef device, CFStringRef key, long fallback) {
    CFTypeRef prop = IOHIDDeviceGetProperty(device, key);
    if (prop && CFGetTypeID(prop) == CFNumberGetTypeID()) {
        long v = fallback;
        CFNumberGetValue((CFNumberRef)prop, kCFNumberLongType, &v);
        return v;
    }
    return fallback;
}

// RAW input report callback. `report` is the report payload; `reportID` is the
// numbered-report ID (0 if the device doesn't use numbered reports). NOTE: the
// reportID byte is NOT included in `report` — prepend it yourself if your
// simulated device uses numbered reports.
//
// Each line carries two timestamps taken at callback entry:
//   ts=<sec.usec>  CLOCK_REALTIME (wall clock) — correlate with a sender's
//                  wall-clock send timestamps on the same machine.
//   dt=<usec>      delta from the previous report, from CLOCK_MONOTONIC_RAW —
//                  inter-report spacing immune to NTP slew.
static void InputReportCallback(void *context, IOReturn result, void *sender,
                                IOHIDReportType type, uint32_t reportID,
                                uint8_t *report, CFIndex reportLength) {
    (void)context; (void)result; (void)sender; (void)type;

    static uint64_t prev_mono_ns = 0;
    struct timespec wall, mono;
    clock_gettime(CLOCK_REALTIME, &wall);
    clock_gettime(CLOCK_MONOTONIC_RAW, &mono);
    uint64_t mono_ns = (uint64_t)mono.tv_sec * 1000000000ull + (uint64_t)mono.tv_nsec;
    long dt_us = prev_mono_ns ? (long)((mono_ns - prev_mono_ns) / 1000ull) : -1;
    prev_mono_ns = mono_ns;

    // ---- Hook your forwarding here. For now just dump hex. ----
    printf("report ts=%ld.%06ld dt=%ld id=%u len=%ld:",
           (long)wall.tv_sec, (long)(wall.tv_nsec / 1000), dt_us,
           reportID, (long)reportLength);
    for (CFIndex i = 0; i < reportLength; i++) printf(" %02X", report[i]);
    printf("\n");
    fflush(stdout);

    // forward_to_simulated_device(reportID, report, reportLength);
}

static void DeviceMatchedCallback(void *context, IOReturn result,
                                  void *sender, IOHIDDeviceRef device) {
    (void)context; (void)result; (void)sender;

    IOReturn r = IOHIDDeviceOpen(device, kIOHIDOptionsTypeSeizeDevice);
    if (r != kIOReturnSuccess) {
        fprintf(stderr, "IOHIDDeviceOpen(seize) failed: 0x%08X\n", r);
        return;
    }

    // Optional but recommended: grab the report descriptor so your simulated
    // device can present an identical one. Then forwarded reports are valid
    // byte-for-byte.
    CFTypeRef desc = IOHIDDeviceGetProperty(device, CFSTR(kIOHIDReportDescriptorKey));
    if (desc && CFGetTypeID(desc) == CFDataGetTypeID()) {
        CFDataRef data = (CFDataRef)desc;
        CFIndex len = CFDataGetLength(data);
        printf("report descriptor (%ld bytes):", (long)len);
        const uint8_t *bytes = CFDataGetBytePtr(data);
        for (CFIndex i = 0; i < len; i++) printf(" %02X", bytes[i]);
        printf("\n");
    } else {
        printf("(report descriptor not available via property)\n");
    }

    // Size the receive buffer from the device's max input report size.
    long maxLen = GetDeviceLongProperty(device, CFSTR(kIOHIDMaxInputReportSizeKey), 64);
    if (maxLen <= 0) maxLen = 64;

    uint8_t *buf = (uint8_t *)malloc((size_t)maxLen);
    if (!buf) { fprintf(stderr, "malloc failed\n"); return; }

    // The buffer must stay alive for the life of the registration. Leaking it
    // here is fine for a single-device test tool; track it if you support many.
    IOHIDDeviceRegisterInputReportCallback(device, buf, maxLen,
                                           InputReportCallback, NULL);

    printf("Device seized; raw reports routed here (buf=%ld bytes).\n", maxLen);
    fflush(stdout);
}

static void DeviceRemovedCallback(void *context, IOReturn result,
                                  void *sender, IOHIDDeviceRef device) {
    (void)context; (void)result; (void)sender;
    printf("Device removed.\n");
    fflush(stdout);
}

static void HandleSignal(int sig) {
    (void)sig;
    if (gManager) IOHIDManagerClose(gManager, kIOHIDOptionsTypeNone);
    printf("\nReleased device. Bye.\n");
    exit(0);
}

int main(void) {
    signal(SIGINT, HandleSignal);

    gManager = IOHIDManagerCreate(kCFAllocatorDefault, kIOHIDOptionsTypeNone);
    if (!gManager) { fprintf(stderr, "IOHIDManagerCreate failed\n"); return 1; }

    CFMutableDictionaryRef match = CreateMatchingDict(kVendorID, kProductID);
    IOHIDManagerSetDeviceMatching(gManager, match);
    if (match) CFRelease(match);

    IOHIDManagerRegisterDeviceMatchingCallback(gManager, DeviceMatchedCallback, NULL);
    IOHIDManagerRegisterDeviceRemovalCallback(gManager, DeviceRemovedCallback, NULL);
    IOHIDManagerScheduleWithRunLoop(gManager, CFRunLoopGetCurrent(),
                                    kCFRunLoopDefaultMode);

    IOReturn r = IOHIDManagerOpen(gManager, kIOHIDOptionsTypeNone);
    if (r != kIOReturnSuccess) {
        fprintf(stderr, "IOHIDManagerOpen failed: 0x%08X\n", r);
        return 1;
    }

    printf("Waiting for device VID=0x%04lX PID=0x%04lX ... (Ctrl-C to quit)\n",
           kVendorID, kProductID);
    fflush(stdout);

    CFRunLoopRun();
    return 0;
}
