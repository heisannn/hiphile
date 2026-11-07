// Writes the expected files: samples of what r8brain-free-src 7.7 itself makes of
// the input that upstream_reference.rs also uses, one route through the library
// per line. From this folder, with that library's checkout in R8B:
//
//   clang++ -std=c++17 -O2 -ffp-contract=off -I"$R8B" generate.cpp -o /tmp/generate
//   /tmp/generate > expected.txt
//
// The same with -DR8B_EXTFFT=1 writes expected_extended_fft.txt, and with
// -DR8B_FLTTEST=1 expected_filter_test.txt.

#if R8B_FLTTEST
namespace r8b {
int InterpFilterFracs = -1;
}
#endif

#include "CDSPResampler.h"

#include <cstdint>
#include <cstdio>
#include <vector>

struct Route {
    double source, target, transition_band, attenuation;
    bool is_minimum_phase;
    int filter_fractions = -1;
};

static const Route ROUTES[] = {
#if R8B_FLTTEST
    // Each interpolates, as this mode never steps wholly
    {44100, 48000, 2, 180.15, false},
    {44100, 48000, 2, 180.15, false, 64},
    {44100, 48000, 45, 49, false, 7},
    {44100, 47999.5, 2, 180.15, false, 1024},
    {96000, 44100, 2, 180.15, false, 256},
    {44100, 96000, 2, 218, false, 65536},
    {44100, 768000, 2, 180.15, false, 5},
    {768000, 44101, 2, 180.15, false, 6},
    {48000, 44100, 2, 180.15, true, 512},
#else
    // One step at a common ratio
    {48000, 24000, 2, 180.15, false},
    {48000, 16000, 2, 180.15, false},
    {48000, 32000, 2, 180.15, false},
    {32000, 48000, 2, 180.15, false},
    {48000, 36000, 2, 180.15, false},
    // Whole upsampling, with half-band steps
    {44100, 88200, 2, 180.15, false},
    {44100, 705600, 2, 180.15, false},
    {44100, 132300, 2, 180.15, false},
    {44100, 1058400, 2, 180.15, false},
    // Interpolation, by whole steps and not, with steps after it
    {44100, 48000, 2, 180.15, false},
    {44100, 96000, 2, 180.15, false},
    {44100, 768000, 2, 180.15, false},
    {11127, 48000, 2, 180.15, false},
    {44100, 47999.5, 2, 180.15, false},
    {44100, 192003, 2, 180.15, false},
    // Slight downsampling
    {48000, 44100, 2, 180.15, false},
    {48000, 47999.5, 2, 180.15, false},
    // Downsampling, with half-band steps, by whole factors and not
    {96000, 44100, 2, 180.15, false},
    {768000, 44100, 2, 180.15, false},
    {384000, 48000, 2, 180.15, false},
    {576000, 48000, 2, 180.15, false},
    {768000, 44101, 2, 180.15, false},
    {192000, 61000, 2, 180.15, false},
    // Ends of the ranges of quality
    {44100, 48000, 0.5, 218, false},
    {44100, 48000, 45, 49, false},
    {768000, 48000, 2, 218, false},
    {44100, 705600, 2, 49, false},
    // Minimum phase
    {44100, 48000, 2, 180.15, true},
    {44100, 132300, 2, 180.15, true},
    {192000, 44100, 2, 180.15, true},
    {44100, 47999.5, 2, 180.15, true},
#endif
};

static const int INPUT_LEN = 60000;
static const int SAMPLES_PER_ROUTE = 32;

int main() {
    std::vector<double> input(INPUT_LEN);
    uint64_t state = 0x9E3779B97F4A7C15ull;
    for (double& sample : input) {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        sample = (double)(state >> 11) * (1.0 / 9007199254740992.0) - 1.0;
    }

    for (const Route& route : ROUTES) {
#if R8B_FLTTEST
        r8b::InterpFilterFracs = route.filter_fractions;
#endif
        r8b::CDSPResampler resampler(
            route.source, route.target, INPUT_LEN, route.transition_band, route.attenuation,
            route.is_minimum_phase ? r8b::fprMinPhase : r8b::fprLinearPhase);
        const long before = resampler.getInLenBeforeOutPos(0);
        double* output;
        const int count = resampler.process(input.data(), INPUT_LEN, output);
        printf("%.17g %.17g %.17g %.17g %d %d %ld %d %.17g", route.source, route.target,
               route.transition_band, route.attenuation, (int)route.is_minimum_phase,
               route.filter_fractions, before, count, resampler.getLatencyFrac());
        for (int i = 0; count > 0 && i < SAMPLES_PER_ROUTE; i++) {
            printf(" %.17g", output[(long)i * (count - 1) / (SAMPLES_PER_ROUTE - 1)]);
        }
        printf("\n");
    }
}
