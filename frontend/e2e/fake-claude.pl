#!/usr/bin/env perl
# A stand-in for Claude Code in the browser tests: each message the owner
# sends becomes one short turn that ends without calling a tool, except that a
# message holding `e2e-bash` first asks to run Bash, one holding
# `e2e-work` searches the web for some seconds, one holding
# `e2e-long-bash` to run many lines of it, and one holding `e2e-plan`
# tells of a plan and writes for some seconds.
#
# Asked once for a dream plan (--json-schema with the dream's prompt), it
# forgets the long-term item whose id starts with `e2e-done`, so a night with
# a commit and a reason can be seen; asked to tidy records, it makes a
# task of those titled `e2e-todo` and doubts those titled `e2e-stale`; any
# other one-shot call gets no answer.
use strict;
use warnings;
$| = 1;
if (grep { $_ eq "--json-schema" } @ARGV) {
    my $prompt = do { local $/; <STDIN> } // "";
    if ($prompt =~ /^Records to look at/) {
        my @records;
        while ($prompt =~ /<record>.*?\nid: (\w+)\n.*?\n# (e2e-todo|e2e-stale).*?<\/record>/gs) {
            my ($id, $verdict, $kind) = ($1, $2 eq "e2e-todo" ? "task" : "suspect", "memos");
            push @records, qq({"record":"records/$kind/$id","verdict":"$verdict","reason":"e2e-why"});
        }
        my $answer = '{"records":[' . join(",", @records) . ']}';
        print qq({"type":"result","subtype":"success","is_error":false,"result":"","structured_output":$answer,"usage":{"input_tokens":1,"output_tokens":1}}\n);
        exit 0;
    }
    exit 0 unless $prompt =~ /^Notes in the inbox:/;
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
    print qq({"type":"control_request","request_id":"e2e-long-bash","request":{"subtype":"can_use_tool","tool_name":"Bash","input":{"command":"), "ls\\n" x 60, qq(rm e2e-last"}}}\n)
      if $line =~ /e2e-long-bash/;
    if ($line =~ /e2e-work/) {
        sleep 2;
        print qq({"type":"assistant","message":{"id":"w","content":[{"type":"tool_use","id":"w1","name":"WebSearch","input":{"query":"x"}}]}}\n);
        sleep 4;
        print qq({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"w1","content":"x"}]}}\n);
        sleep 2;
    }
    if ($line =~ /e2e-plan/) {
        print qq({"type":"assistant","message":{"id":"p","content":[{"type":"text","text":"まず読む"}]}}\n);
        print qq({"type":"assistant","message":{"id":"p","content":[{"type":"tool_use","name":"TodoWrite","input":{"todos":[{"content":"読む","status":"completed"},{"content":"書く","status":"in_progress"}]}}]}}\n);
        sleep 6;
        print qq({"type":"assistant","message":{"id":"p","content":[{"type":"text","text":"書き終えた"}]}}\n);
    }
    print qq({"type":"result","subtype":"success","is_error":false,"session_id":"e2e","usage":{"input_tokens":1,"output_tokens":1}}\n);
}
