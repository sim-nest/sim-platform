The unchanged portable requirement binds the monotonic and config-directory
services and selects the declared `platform/service/none` substitute for the
optional GPIO service. Removing the required config-directory offer refuses the
whole request; the resolver returns no partial binding.
