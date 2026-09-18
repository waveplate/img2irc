ARG MANYLINUX_IMAGE=quay.io/pypa/manylinux_2_28_x86_64
FROM ${MANYLINUX_IMAGE}

# OpenSSL's source build needs IPC::Cmd and Time::Piece, which the base image's
# minimal Perl installation does not provide.
RUN dnf install -y perl-core && dnf clean all
