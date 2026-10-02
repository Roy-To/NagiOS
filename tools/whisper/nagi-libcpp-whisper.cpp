// Nagi-owned target libc++ ABI support for the pinned Whisper CPU path.
// Compile against target libc++ headers; do not link a host runtime/libc++.a.

#include <string>

namespace std {
inline namespace __1 {

template <typename Unsigned>
static string nagi_unsigned_to_string(Unsigned magnitude, bool negative) {
  char buffer[sizeof(Unsigned) * 3 + 2];
  char *cursor = buffer + sizeof(buffer);
  do {
    *--cursor = static_cast<char>('0' + magnitude % 10U);
    magnitude /= 10U;
  } while (magnitude != 0U);
  if (negative)
    *--cursor = '-';
  return string(cursor,
                static_cast<string::size_type>(buffer + sizeof(buffer) - cursor));
}

string to_string(int value) {
  const bool negative = value < 0;
  unsigned int magnitude = negative ? 0U - static_cast<unsigned int>(value)
                                    : static_cast<unsigned int>(value);
  return nagi_unsigned_to_string(magnitude, negative);
}

string to_string(long value) {
  const bool negative = value < 0;
  const unsigned long magnitude = negative ? 0UL - static_cast<unsigned long>(value)
                                           : static_cast<unsigned long>(value);
  return nagi_unsigned_to_string(magnitude, negative);
}

string to_string(unsigned int value) {
  return nagi_unsigned_to_string(value, false);
}

template basic_string<char>::~basic_string();
template void basic_string<char>::__init(
    const char *, basic_string<char>::size_type);
template basic_string<char>& basic_string<char>::assign(const char*);
template basic_string<char>& basic_string<char>::assign(
    const char*, basic_string<char>::size_type);
template basic_string<char>& basic_string<char>::append(const char*);
template basic_string<char>& basic_string<char>::append(
    const char*, basic_string<char>::size_type);
template basic_string<char>& basic_string<char>::insert(
    basic_string<char>::size_type, const char*);
template basic_string<char>& basic_string<char>::insert(
    basic_string<char>::size_type, const char*, basic_string<char>::size_type);
template basic_string<char>& basic_string<char>::operator=(char);
template basic_string<char>& basic_string<char>::operator=(const basic_string<char>&);
template void basic_string<char>::__grow_by_and_replace(
    basic_string<char>::size_type, basic_string<char>::size_type,
    basic_string<char>::size_type, basic_string<char>::size_type,
    basic_string<char>::size_type, basic_string<char>::size_type, const char*);
template int basic_string<char>::compare(
    basic_string<char>::size_type, basic_string<char>::size_type, const char*) const;
template int basic_string<char>::compare(
    basic_string<char>::size_type, basic_string<char>::size_type,
    const char*, basic_string<char>::size_type) const;
template basic_string<char> operator+<char, char_traits<char>, allocator<char>>(
    const char*, const basic_string<char>&);

} // namespace __1
} // namespace std
