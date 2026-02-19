# frozen_string_literal: true

module Recollect
  class MarkdownChunker
    # Approximate tokens by words * 1.3
    TOKEN_RATIO = 1.3
    DEFAULT_MAX_TOKENS = 900
    OVERLAP_PERCENT = 0.15

    def self.chunk(content, max_tokens: DEFAULT_MAX_TOKENS)
      return [content] if content.nil? || content.empty?
      
      # Estimate total tokens
      total_tokens = estimate_tokens(content)
      return [content] if total_tokens <= max_tokens

      chunks = []
      lines = content.lines
      current_chunk = []
      current_tokens = 0
      
      lines.each do |line|
        line_tokens = estimate_tokens(line)
        
        # If adding this line exceeds limit and we have content, finalize current chunk
        if current_tokens + line_tokens > max_tokens && !current_chunk.empty?
          chunks << current_chunk.join
          
          # Overlap: keep some lines for context
          overlap_target = (max_tokens * OVERLAP_PERCENT).to_i
          overlap_chunk = []
          overlap_tokens = 0
          
          current_chunk.reverse_each do |l|
            l_tokens = estimate_tokens(l)
            break if overlap_tokens + l_tokens > overlap_target
            overlap_chunk.unshift(l)
            overlap_tokens += l_tokens
          end
          
          current_chunk = overlap_chunk
          current_tokens = overlap_tokens
        end
        
        current_chunk << line
        current_tokens += line_tokens
      end
      
      chunks << current_chunk.join unless current_chunk.empty?
      chunks
    end

    def self.estimate_tokens(text)
      (text.split(/\s+/).size * TOKEN_RATIO).to_i
    end
  end
end
