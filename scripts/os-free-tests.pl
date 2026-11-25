#!/usr/bin/env perl
# Print the `--skip` arguments that leave out the tests no OS can make a
# difference to, so that macOS, whose runners are few, runs the rest. Nothing
# is listed by hand: a test is left out only when its file, and the one it is
# a test of, say nothing that reaches the OS. What is not certain is run.
#
# Fails when code is compiled for one OS only, as Linux alone lints it all.
use strict;
use warnings;
use File::Find;

my $touches = qr{
    cfg!?\([^)]*(?:unix|windows|target_os|macos)
  | std::os:: | \bCommand\b | \bprocess:: | \bfs:: | symlink | canonicalize
  | env::(?:temp_dir|current_dir|current_exe|var) | (?:Unix|Tcp|Udp)(?:Stream|Listener|Socket)
  | modified\(\) | libc | rustix | \bgit\b | perl | \.exists\(\) | is_file\(\) | is_dir\(\) | \bkill | signal:: | sandbox
  | \bInstant\b
}x;

# The code without its comments, which may speak of what it does not touch.
sub code {
    open my $in, '<', $_[0] or return '';
    return join '', map { s{//.*}{}r } <$in>;
}

my (%tests, %all);    # module path with a trailing `::`, to whether it is left out
find({ no_chdir => 1, wanted => sub {
    return unless /\.rs$/;
    my ($dir, $file) = m{^(crates/[^/]+/(?:src|tests/[^/]+))/(.*)\.rs$} or return;
    my $text = code($_);
    if ($text =~ /#\[cfg\([^]]*(?:target_os|target_family|target_vendor)/) {
        die "$_: code for one OS is not linted by CI: lint on that OS as well (just ci-macos)\n";
    }
    return unless $text =~ /#\[(?:tokio::)?test\b/;
    (my $module = $file) =~ s{/}{::}g;
    $module =~ s/::mod$//;
    my $integration = $dir =~ m{/tests/};
    my $of = $module;
    if ($integration) {
        $module .= '::';
    } elsif ($of =~ s/::tests$//) {
        $module = "${of}::tests::";
    } else {
        $module .= '::tests::';
    }
    (my $at = "$dir/$of") =~ s{::}{/}g;
    # These start programs, wherever the code that does lies.
    my $starts = $of =~ /^(?:workers|claude|mcp)(?:::|$)/;
    my $touched = $starts || grep { code($_) =~ $touches } $_, "$at.rs", "$at/mod.rs";
    $all{$module} = 1;
    # Each crate has modules of its own, which may share a name.
    $tests{$module} = exists $tests{$module} ? $tests{$module} && !$touched : !$touched;
} }, 'crates');

my @skipped = sort grep { $tests{$_} } keys %tests;
# libtest leaves out whatever contains the words, so what also names a test to
# run is not left out, until nothing changes.
my %run = map { $_ => 1 } grep { !$tests{$_} } keys %tests;
while (my @both = grep { my $s = $_; grep { index($_, $s) >= 0 } keys %run } @skipped) {
    @run{@both} = (1) x @both;
    my %both = map { $_ => 1 } @both;
    @skipped = grep { !$both{$_} } @skipped;
}
print STDERR "left to Linux: @skipped\n";
print map { "--skip $_ " } @skipped;
