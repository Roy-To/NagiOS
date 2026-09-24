// The pinned libc++ ABI declares these integer sort specializations as
// extern-template symbols, while Nagi does not link libc++.a. Keep the exact
// libc++ ABI names and provide an allocation-free heapsort for each used type.
// The source intentionally forward-declares only the ABI types and function
// template: including <algorithm> would reintroduce the extern-template
// declaration before these Nagi-owned definitions.

namespace std {
inline namespace __1 {

template <class Left, class Right> struct __less;
template <class Compare, class Iterator>
void __sort(Iterator, Iterator, Compare);

template <class T> static void nagi_sift_down(T *values, __SIZE_TYPE__ root,
                                              __SIZE_TYPE__ end) {
  while (root < end / 2) {
    __SIZE_TYPE__ child = root * 2 + 1;
    if (child + 1 < end && values[child] < values[child + 1])
      ++child;
    if (!(values[root] < values[child]))
      return;
    T temporary = values[root];
    values[root] = values[child];
    values[child] = temporary;
    root = child;
  }
}

template <class T> static void nagi_sort(T *first, T *last) {
  if (first == last)
    return;
  const __SIZE_TYPE__ length = static_cast<__SIZE_TYPE__>(last - first);
  if (length < 2)
    return;

  for (__SIZE_TYPE__ start = length / 2; start != 0; --start)
    nagi_sift_down(first, start - 1, length);
  for (__SIZE_TYPE__ end = length; end > 1; --end) {
    T temporary = first[0];
    first[0] = first[end - 1];
    first[end - 1] = temporary;
    nagi_sift_down(first, 0, end - 1);
  }
}

#define NAGI_LIBCPP_SORT(Type)                                                 \
  template <>                                                                  \
  void __sort<__less<Type, Type> &, Type *>(Type *first, Type *last,           \
                                            __less<Type, Type> &) {            \
    nagi_sort(first, last);                                                    \
  }

NAGI_LIBCPP_SORT(signed char)
NAGI_LIBCPP_SORT(int)
NAGI_LIBCPP_SORT(long)
NAGI_LIBCPP_SORT(short)
NAGI_LIBCPP_SORT(unsigned short)
NAGI_LIBCPP_SORT(unsigned char)
NAGI_LIBCPP_SORT(unsigned int)
NAGI_LIBCPP_SORT(unsigned long)

#undef NAGI_LIBCPP_SORT

} // namespace __1
} // namespace std
