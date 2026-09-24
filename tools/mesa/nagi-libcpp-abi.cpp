// Target-owned libc++ ABI entrypoints required by the pinned Servo/MozJS
// build. This translation unit must use the same libc++ headers as the target
// objects; it intentionally does not link libc++.a or a host runtime.

#include <chrono>
#include <cstdint>
#include <string>
#include <thread>

extern "C" int nagi_posix_sleep_ns(std::uint64_t duration);

namespace std {
inline namespace __1 {

namespace this_thread {
void sleep_for(const chrono::nanoseconds &duration) {
  const auto count = duration.count();
  if (count > 0)
    (void)nagi_posix_sleep_ns(static_cast<std::uint64_t>(count));
}
} // namespace this_thread

template basic_string<char> &
basic_string<char>::append(basic_string<char>::size_type, char);

// OTS uses these libc++ string ABI-v1 entrypoints. Instantiate the pinned
// target header implementations directly; do not link a host libc++ archive.
template basic_string<char> &basic_string<char>::assign(const char *);
template basic_string<char> &basic_string<char>::assign(const char *,
                                                        basic_string<char>::size_type);
template void basic_string<char>::resize(basic_string<char>::size_type, char);
template basic_string<char> &basic_string<char>::append(
    const char *, basic_string<char>::size_type);
template basic_string<char> &basic_string<char>::replace(
    basic_string<char>::size_type, basic_string<char>::size_type, const char *,
    basic_string<char>::size_type);

// Servo and MozJS reference libc++'s ABI-v1 growth helper from an
// extern-template instantiation. Provide the real header implementation in
// this target-owned object because Nagi intentionally has no libc++.a.
template void basic_string<char>::__grow_by(
    basic_string<char>::size_type,
    basic_string<char>::size_type,
    basic_string<char>::size_type,
    basic_string<char>::size_type,
    basic_string<char>::size_type,
    basic_string<char>::size_type);

} // namespace __1
} // namespace std
