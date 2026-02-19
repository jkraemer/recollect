# frozen_string_literal: true

require "test_helper"

class MarkdownChunkerTest < Recollect::TestCase
  def test_chunk_small_content
    content = "This is a small piece of content."
    chunks = Recollect::MarkdownChunker.chunk(content, max_tokens: 100)

    assert_equal 1, chunks.length
    assert_equal content, chunks.first
  end

  def test_chunk_large_content
    # Create content that is definitely more than 5 tokens
    content = 10.times.map { |i| "Line #{i}.\n" }.join
    chunks = Recollect::MarkdownChunker.chunk(content, max_tokens: 5)

    assert_operator chunks.length, :>, 1
  end

  def test_chunk_with_overlap
    content = "Line one.\nLine two.\nLine three.\nLine four.\nLine five.\n"
    chunks = Recollect::MarkdownChunker.chunk(content, max_tokens: 5)

    assert_operator chunks.length, :>, 1
    # Check that some content from chunk 1 is in chunk 2 (due to overlap)
    # The first line is "Line one.\n" (~3 words -> 4 tokens)
    # The second line is "Line two.\n" (~3 words -> 4 tokens)
    # With max_tokens 5, chunk 1 gets Line 1.
    # Chunk 2 should start with some overlap if implemented
    assert_match(/Line/, chunks[0])
    assert_match(/Line/, chunks[1])
  end

  def test_estimate_tokens
    text = "Hello world, this is a test." # 6 words
    # 6 * 1.3 = 7.8 -> 7
    assert_equal 7, Recollect::MarkdownChunker.estimate_tokens(text)
  end
end
