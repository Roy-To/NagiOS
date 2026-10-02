// Nagi-owned libc++ ABI support for the pinned ggml CPU feature formatter.
// Compile against the configured target libc++ headers; do not link a host
// runtime or import libc++.a.

#include <string>

namespace std {
inline namespace __1 {

string to_string(int value) {
  char buffer[sizeof(int) * 3 + 2];
  char *cursor = buffer + sizeof(buffer);
  const bool negative = value < 0;
  unsigned int magnitude = negative ? 0U - static_cast<unsigned int>(value)
                                    : static_cast<unsigned int>(value);

  do {
    *--cursor = static_cast<char>('0' + magnitude % 10U);
    magnitude /= 10U;
  } while (magnitude != 0U);
  if (negative)
    *--cursor = '-';

  return string(cursor,
                static_cast<string::size_type>(buffer + sizeof(buffer) - cursor));
}

template basic_string<char>::~basic_string();
template void basic_string<char>::__init(
    const char *, basic_string<char>::size_type);

} // namespace __1
} // namespace std

extern "C" int nagi_m20_libcpp_int_to_string_smoke() {
  const std::string negative = std::to_string(-214);
  const int minimum = (-2147483647 - 1);
  const std::string minimum_text = std::to_string(minimum);
  const bool negative_ok = negative.size() == 4 && negative[0] == '-' &&
                           negative[1] == '2' && negative[2] == '1' &&
                           negative[3] == '4';
  const bool minimum_ok = minimum_text.size() == 11 && minimum_text[0] == '-' &&
                          minimum_text[1] == '2' && minimum_text[10] == '8';
  return negative_ok && minimum_ok ? 0 : 1;
}
