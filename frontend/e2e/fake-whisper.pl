#!/usr/bin/env perl
# Stands in for whisper.cpp's server: answers on the port it is given that it
# is ready.
use strict;
use warnings;
use IO::Socket::INET;

my %args;
while (@ARGV >= 2) {
    my ($name, $value) = splice @ARGV, 0, 2;
    $args{$name} = $value;
}
my $listener = IO::Socket::INET->new(
    LocalAddr => $args{'--host'},
    LocalPort => $args{'--port'},
    Listen    => 5,
    ReuseAddr => 1,
) or die "cannot listen: $!\n";
while (my $asked = $listener->accept) {
    while (defined(my $line = <$asked>)) {
        last if $line =~ /^\r?\n$/;
    }
    print $asked "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok";
    close $asked;
}
