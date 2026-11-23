#!/usr/bin/env perl
# A stand-in for Claude Code in the browser tests: each message the owner
# sends becomes one short turn that ends without calling a tool, except that a
# message holding `e2e-bash` first asks to run Bash.
#
# Asked once for a dream plan (--json-schema with the dream's prompt), it
# forgets the long-term item whose id starts with `e2e-done`, so a night with
# a commit and a reason can be seen; any other one-shot call gets no answer.
use strict;
use warnings;
$| = 1;
if (grep { $_ eq "--json-schema" } @ARGV) {
    my $prompt = do { local $/; <STDIN> } // "";
    exit 0 unless $prompt =~ /^Episodic notes:/;
    my @edits = map { qq({"op":"forget","id":"$_","reason":"e2eで済んだ"}) } ($prompt =~ /\{id: (e2e-done[\w-]*)/g);
    my $plan = '{"edits":[' . join(",", @edits) . '],"settled":[]}';
    print qq({"type":"result","subtype":"success","is_error":false,"result":"","structured_output":$plan,"usage":{"input_tokens":1,"output_tokens":1}}\n);
    exit 0;
}
if (grep { $_ eq "--no-session-persistence" } @ARGV) {
    exit 0 unless (<STDIN> // "") =~ /"initialize"/;
    my @models = (
        [default => "Default"], [opus => "Opus 5.5"], [sonnet => "Sonnet 5.5"],
        [haiku => "Haiku 5.5"], ["claude-opus-5" => "Opus 5"],
    );
    my $models = join ",", map { qq({"value":"$_->[0]","displayName":"$_->[1]"}) } @models;
    print qq({"type":"control_response","response":{"subtype":"success","response":{"models":[$models]}}}\n);
    exit 0;
}
my $started = 0;
while (my $line = <STDIN>) {
    next unless $line =~ /"type":"user"/;
    print qq({"type":"system","subtype":"init","session_id":"e2e","tools":["Bash","Read"]}\n) unless $started++;
    print qq({"type":"control_request","request_id":"e2e-bash","request":{"subtype":"can_use_tool","tool_name":"Bash","input":{"command":"ls"}}}\n)
      if $line =~ /e2e-bash/;
    print qq({"type":"result","subtype":"success","is_error":false,"session_id":"e2e","usage":{"input_tokens":1,"output_tokens":1}}\n);
}
