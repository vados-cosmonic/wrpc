// Generated by `wit-bindgen-wrpc-go` 0.5.0. DO NOT EDIT!
package handler

import (
	bytes "bytes"
	context "context"
	binary "encoding/binary"
	errors "errors"
	fmt "fmt"
	wrpc "github.com/bytecodealliance/wrpc/go"
	io "io"
	slog "log/slog"
	math "math"
	sync "sync"
	atomic "sync/atomic"
)

type Req struct {
	Numbers wrpc.ReceiveCompleter[[]uint64]
	Bytes   wrpc.ReadCompleter
}

func (v *Req) String() string { return "Req" }

func (v *Req) WriteToIndex(w wrpc.ByteWriter) (func(wrpc.IndexWriter) error, error) {
	writes := make(map[uint32]func(wrpc.IndexWriter) error, 2)
	slog.Debug("writing field", "name", "numbers")
	write0, err := func(v wrpc.ReceiveCompleter[[]uint64], w interface {
		io.ByteWriter
		io.Writer
	}) (write func(wrpc.IndexWriter) error, err error) {
		if v.IsComplete() {
			defer func() {
				body, ok := v.(io.Closer)
				if ok {
					if cErr := body.Close(); cErr != nil {
						if err == nil {
							err = fmt.Errorf("failed to close ready stream: %w", cErr)
						} else {
							slog.Warn("failed to close ready stream", "err", cErr)
						}
					}
				}
			}()
			slog.Debug("writing stream `stream::ready` status byte")
			if err = w.WriteByte(1); err != nil {
				return nil, fmt.Errorf("failed to write `stream::ready` byte: %w", err)
			}
			slog.Debug("receiving ready stream contents")
			vs, err := v.Receive()
			if err != nil && err != io.EOF {
				return nil, fmt.Errorf("failed to receive ready stream contents: %w", err)
			}
			if err != io.EOF && len(vs) > 0 {
				for {
					chunk, err := v.Receive()
					if err != nil && err != io.EOF {
						return nil, fmt.Errorf("failed to receive ready stream contents: %w", err)
					}
					if len(chunk) > 0 {
						vs = append(vs, chunk...)
					}
					if err == io.EOF {
						break
					}
				}
			}
			slog.Debug("writing ready stream contents", "len", len(vs))
			write, err := func(v []uint64, w interface {
				io.ByteWriter
				io.Writer
			}) (write func(wrpc.IndexWriter) error, err error) {
				n := len(v)
				if n > math.MaxUint32 {
					return nil, fmt.Errorf("list length of %d overflows a 32-bit integer", n)
				}
				if err = func(v int, w io.Writer) error {
					b := make([]byte, binary.MaxVarintLen32)
					i := binary.PutUvarint(b, uint64(v))
					slog.Debug("writing list length", "len", n)
					_, err = w.Write(b[:i])
					return err
				}(n, w); err != nil {
					return nil, fmt.Errorf("failed to write list length of %d: %w", n, err)
				}
				slog.Debug("writing list elements")
				writes := make(map[uint32]func(wrpc.IndexWriter) error, n)
				for i, e := range v {
					write, err := (func(wrpc.IndexWriter) error)(nil), func(v uint64, w io.Writer) (err error) {
						b := make([]byte, binary.MaxVarintLen64)
						i := binary.PutUvarint(b, uint64(v))
						slog.Debug("writing u64")
						_, err = w.Write(b[:i])
						return err
					}(e, w)
					if err != nil {
						return nil, fmt.Errorf("failed to write list element %d: %w", i, err)
					}
					if write != nil {
						writes[uint32(i)] = write
					}
				}
				if len(writes) > 0 {
					return func(w wrpc.IndexWriter) error {
						var wg sync.WaitGroup
						var wgErr atomic.Value
						for index, write := range writes {
							wg.Add(1)
							w, err := w.Index(index)
							if err != nil {
								return fmt.Errorf("failed to index writer: %w", err)
							}
							write := write
							go func() {
								defer wg.Done()
								if err := write(w); err != nil {
									wgErr.Store(err)
								}
							}()
						}
						wg.Wait()
						err := wgErr.Load()
						if err == nil {
							return nil
						}
						return err.(error)
					}, nil
				}
				return nil, nil
			}(vs, w)
			if err != nil {
				return nil, fmt.Errorf("failed to write ready stream contents: %w", err)
			}
			return write, nil
		} else {
			slog.Debug("writing stream `stream::pending` status byte")
			if err := w.WriteByte(0); err != nil {
				return nil, fmt.Errorf("failed to write `stream::pending` byte: %w", err)
			}
			return func(w wrpc.IndexWriter) (err error) {
				defer func() {
					body, ok := v.(io.Closer)
					if ok {
						if cErr := body.Close(); cErr != nil {
							if err == nil {
								err = fmt.Errorf("failed to close pending stream: %w", cErr)
							} else {
								slog.Warn("failed to close pending stream", "err", cErr)
							}
						}
					}
				}()
				var wg sync.WaitGroup
				var wgErr atomic.Value
				var total uint32
				for {
					var end bool
					slog.Debug("receiving outgoing pending stream contents")
					chunk, err := v.Receive()
					n := len(chunk)
					if n == 0 || err == io.EOF {
						end = true
						slog.Debug("outgoing pending stream reached EOF")
					} else if err != nil {
						return fmt.Errorf("failed to receive outgoing pending stream chunk: %w", err)
					}
					if n > math.MaxUint32 {
						return fmt.Errorf("outgoing pending stream chunk length of %d overflows a 32-bit integer", n)
					}
					if math.MaxUint32-uint32(n) < total {
						return errors.New("total outgoing pending stream element count would overflow a 32-bit unsigned integer")
					}
					slog.Debug("writing pending stream chunk length", "len", n)
					if err = wrpc.WriteUint32(uint32(n), w); err != nil {
						return fmt.Errorf("failed to write pending stream chunk length of %d: %w", n, err)
					}
					for _, v := range chunk {
						slog.Debug("writing pending stream element", "i", total)
						write, err := (func(wrpc.IndexWriter) error)(nil), func(v uint64, w io.Writer) (err error) {
							b := make([]byte, binary.MaxVarintLen64)
							i := binary.PutUvarint(b, uint64(v))
							slog.Debug("writing u64")
							_, err = w.Write(b[:i])
							return err
						}(v, w)
						if err != nil {
							return fmt.Errorf("failed to write pending stream chunk element %d: %w", total, err)
						}
						if write != nil {
							wg.Add(1)
							w, err := w.Index(total)
							if err != nil {
								return fmt.Errorf("failed to index writer: %w", err)
							}
							go func() {
								defer wg.Done()
								if err := write(w); err != nil {
									wgErr.Store(err)
								}
							}()
						}
						total++
					}
					if end {
						if err := w.WriteByte(0); err != nil {
							return fmt.Errorf("failed to write pending stream end byte: %w", err)
						}
						wg.Wait()
						err := wgErr.Load()
						if err == nil {
							return nil
						}
						return err.(error)
					}
				}
			}, nil
		}
	}(v.Numbers, w)
	if err != nil {
		return nil, fmt.Errorf("failed to write `numbers` field: %w", err)
	}
	if write0 != nil {
		writes[0] = write0
	}
	slog.Debug("writing field", "name", "bytes")
	write1, err := func(v wrpc.ReadCompleter, w interface {
		io.ByteWriter
		io.Writer
	}) (write func(wrpc.IndexWriter) error, err error) {
		if v.IsComplete() {
			defer func() {
				body, ok := v.(io.Closer)
				if ok {
					if cErr := body.Close(); cErr != nil {
						if err == nil {
							err = fmt.Errorf("failed to close ready byte stream: %w", cErr)
						} else {
							slog.Warn("failed to close ready byte stream", "err", cErr)
						}
					}
				}
			}()
			slog.Debug("writing byte stream `stream::ready` status byte")
			if err = w.WriteByte(1); err != nil {
				return nil, fmt.Errorf("failed to write `stream::ready` byte: %w", err)
			}
			slog.Debug("reading ready byte stream contents")
			var buf bytes.Buffer
			var n int64
			n, err = io.Copy(&buf, v)
			if err != nil {
				return nil, fmt.Errorf("failed to read ready byte stream contents: %w", err)
			}
			slog.Debug("writing ready byte stream contents", "len", n)
			if err = wrpc.WriteByteList(buf.Bytes(), w); err != nil {
				return nil, fmt.Errorf("failed to write ready byte stream contents: %w", err)
			}
			return nil, nil
		} else {
			slog.Debug("writing byte stream `stream::pending` status byte")
			if err = w.WriteByte(0); err != nil {
				return nil, fmt.Errorf("failed to write `stream::pending` byte: %w", err)
			}
			return func(w wrpc.IndexWriter) (err error) {
				defer func() {
					body, ok := v.(io.Closer)
					if ok {
						if cErr := body.Close(); cErr != nil {
							if err == nil {
								err = fmt.Errorf("failed to close pending byte stream: %w", cErr)
							} else {
								slog.Warn("failed to close pending byte stream", "err", cErr)
							}
						}
					}
				}()
				chunk := make([]byte, 8096)
				for {
					var end bool
					slog.Debug("reading pending byte stream contents")
					n, err := v.Read(chunk)
					if err == io.EOF {
						end = true
						slog.Debug("pending byte stream reached EOF")
					} else if err != nil {
						return fmt.Errorf("failed to read pending byte stream chunk: %w", err)
					}
					if n > math.MaxUint32 {
						return fmt.Errorf("pending byte stream chunk length of %d overflows a 32-bit integer", n)
					}
					slog.Debug("writing pending byte stream chunk length", "len", n)
					if err := wrpc.WriteUint32(uint32(n), w); err != nil {
						return fmt.Errorf("failed to write pending byte stream chunk length of %d: %w", n, err)
					}
					_, err = w.Write(chunk[:n])
					if err != nil {
						return fmt.Errorf("failed to write pending byte stream chunk contents: %w", err)
					}
					if end {
						if err := w.WriteByte(0); err != nil {
							return fmt.Errorf("failed to write pending byte stream end byte: %w", err)
						}
						return nil
					}
				}
			}, nil
		}
	}(v.Bytes, w)
	if err != nil {
		return nil, fmt.Errorf("failed to write `bytes` field: %w", err)
	}
	if write1 != nil {
		writes[1] = write1
	}

	if len(writes) > 0 {
		return func(w wrpc.IndexWriter) error {
			var wg sync.WaitGroup
			var wgErr atomic.Value
			for index, write := range writes {
				wg.Add(1)
				w, err := w.Index(index)
				if err != nil {
					return fmt.Errorf("failed to index writer: %w", err)
				}
				write := write
				go func() {
					defer wg.Done()
					if err := write(w); err != nil {
						wgErr.Store(err)
					}
				}()
			}
			wg.Wait()
			err := wgErr.Load()
			if err == nil {
				return nil
			}
			return err.(error)
		}, nil
	}
	return nil, nil
}

type Handler interface {
	Echo(ctx__ context.Context, r *Req) (wrpc.ReceiveCompleter[[]uint64], wrpc.ReadCompleter, error)
}

func ServeInterface(s wrpc.Server, h Handler) (stop func() error, err error) {
	stops := make([]func() error, 0, 1)
	stop = func() error {
		for _, stop := range stops {
			if err := stop(); err != nil {
				return err
			}
		}
		return nil
	}
	stop0, err := s.Serve("wrpc-examples:streams/handler", "echo", func(ctx context.Context, w wrpc.IndexWriter, r wrpc.IndexReadCloser) error {
		slog.DebugContext(ctx, "reading parameter", "i", 0)
		p0, err := func(r wrpc.IndexReader, path ...uint32) (*Req, error) {
			v := &Req{}
			var err error
			slog.Debug("reading field", "name", "numbers")
			v.Numbers, err = func(r wrpc.IndexReader, path ...uint32) (wrpc.ReceiveCompleter[[]uint64], error) {
				slog.Debug("reading stream status byte")
				status, err := r.ReadByte()
				if err != nil {
					return nil, fmt.Errorf("failed to read stream status byte: %w", err)
				}
				switch status {
				case 0:
					if len(path) > 0 {
						r, err = r.Index(path...)
						if err != nil {
							return nil, fmt.Errorf("failed to index reader: %w", err)
						}
					}
					var total uint32
					return wrpc.NewDecodeReceiver(r, func(r wrpc.IndexReader) ([]uint64, error) {
						slog.Debug("reading pending stream chunk length")
						n, err := func(r io.ByteReader) (uint32, error) {
							var x uint32
							var s uint8
							for i := 0; i < 5; i++ {
								slog.Debug("reading u32 byte", "i", i)
								b, err := r.ReadByte()
								if err != nil {
									if i > 0 && err == io.EOF {
										err = io.ErrUnexpectedEOF
									}
									return x, fmt.Errorf("failed to read u32 byte: %w", err)
								}
								if s == 28 && b > 0x0f {
									return x, errors.New("varint overflows a 32-bit integer")
								}
								if b < 0x80 {
									return x | uint32(b)<<s, nil
								}
								x |= uint32(b&0x7f) << s
								s += 7
							}
							return x, errors.New("varint overflows a 32-bit integer")
						}(r)
						if err != nil {
							return nil, fmt.Errorf("failed to read pending stream chunk length: %w", err)
						}
						if n == 0 {
							return nil, io.EOF
						}
						if math.MaxUint32-n < total {
							return nil, errors.New("total incoming pending stream element count would overflow a 32-bit unsigned integer")
						}
						vs := make([]uint64, n)
						for i := range vs {
							slog.Debug("reading pending stream element", "i", total)
							v, err := func(r io.ByteReader) (uint64, error) {
								var x uint64
								var s uint8
								for i := 0; i < 10; i++ {
									slog.Debug("reading u64 byte", "i", i)
									b, err := r.ReadByte()
									if err != nil {
										if i > 0 && err == io.EOF {
											err = io.ErrUnexpectedEOF
										}
										return x, fmt.Errorf("failed to read u64 byte: %w", err)
									}
									if s == 63 && b > 0x01 {
										return x, errors.New("varint overflows a 64-bit integer")
									}
									if b < 0x80 {
										return x | uint64(b)<<s, nil
									}
									x |= uint64(b&0x7f) << s
									s += 7
								}
								return x, errors.New("varint overflows a 64-bit integer")
							}(r)
							if err != nil {
								return nil, fmt.Errorf("failed to read pending stream chunk element %d: %w", i, err)
							}
							vs[i] = v
							total++
						}
						return vs, nil
					}), nil
				case 1:
					slog.Debug("reading ready stream contents")
					vs, err :=
						func(r wrpc.IndexReader, path ...uint32) ([]uint64, error) {
							var x uint32
							var s uint
							for i := 0; i < 5; i++ {
								slog.Debug("reading list length byte", "i", i)
								b, err := r.ReadByte()
								if err != nil {
									if i > 0 && err == io.EOF {
										err = io.ErrUnexpectedEOF
									}
									return nil, fmt.Errorf("failed to read list length byte: %w", err)
								}
								if b < 0x80 {
									if i == 4 && b > 1 {
										return nil, errors.New("list length overflows a 32-bit integer")
									}
									x = x | uint32(b)<<s
									vs := make([]uint64, x)
									for i := range vs {
										slog.Debug("reading list element", "i", i)
										vs[i], err = func(r io.ByteReader) (uint64, error) {
											var x uint64
											var s uint8
											for i := 0; i < 10; i++ {
												slog.Debug("reading u64 byte", "i", i)
												b, err := r.ReadByte()
												if err != nil {
													if i > 0 && err == io.EOF {
														err = io.ErrUnexpectedEOF
													}
													return x, fmt.Errorf("failed to read u64 byte: %w", err)
												}
												if s == 63 && b > 0x01 {
													return x, errors.New("varint overflows a 64-bit integer")
												}
												if b < 0x80 {
													return x | uint64(b)<<s, nil
												}
												x |= uint64(b&0x7f) << s
												s += 7
											}
											return x, errors.New("varint overflows a 64-bit integer")
										}(r)
										if err != nil {
											return nil, fmt.Errorf("failed to read list element %d: %w", i, err)
										}
									}
									return vs, nil
								}
								x |= uint32(b&0x7f) << s
								s += 7
							}
							return nil, errors.New("list length overflows a 32-bit integer")
						}(r, path...)
					if err != nil {
						return nil, fmt.Errorf("failed to read ready stream contents: %w", err)
					}
					slog.Debug("read ready stream contents", "len", len(vs))
					return wrpc.NewCompleteReceiver(vs), nil
				default:
					return nil, fmt.Errorf("invalid stream status byte %d", status)
				}
			}(r, append(path, 0)...)
			if err != nil {
				return nil, fmt.Errorf("failed to read `numbers` field: %w", err)
			}
			slog.Debug("reading field", "name", "bytes")
			v.Bytes, err = func(r wrpc.IndexReader, path ...uint32) (wrpc.ReadCompleter, error) {
				slog.Debug("reading byte stream status byte")
				status, err := r.ReadByte()
				if err != nil {
					return nil, fmt.Errorf("failed to read byte stream status byte: %w", err)
				}
				switch status {
				case 0:
					if len(path) > 0 {
						r, err = r.Index(path...)
						if err != nil {
							return nil, fmt.Errorf("failed to index reader: %w", err)
						}
					}
					return wrpc.NewByteStreamReader(wrpc.NewPendingByteReader(r)), nil
				case 1:
					slog.Debug("reading ready byte stream contents")
					buf, err :=
						func(r interface {
							io.ByteReader
							io.Reader
						}) ([]byte, error) {
							var x uint32
							var s uint
							for i := 0; i < 5; i++ {
								slog.Debug("reading byte list length", "i", i)
								b, err := r.ReadByte()
								if err != nil {
									if i > 0 && err == io.EOF {
										err = io.ErrUnexpectedEOF
									}
									return nil, fmt.Errorf("failed to read byte list length byte: %w", err)
								}
								if b < 0x80 {
									if i == 4 && b > 1 {
										return nil, errors.New("byte list length overflows a 32-bit integer")
									}
									x = x | uint32(b)<<s
									buf := make([]byte, x)
									slog.Debug("reading byte list contents", "len", x)
									_, err = io.ReadFull(r, buf)
									if err != nil {
										return nil, fmt.Errorf("failed to read byte list contents: %w", err)
									}
									return buf, nil
								}
								x |= uint32(b&0x7f) << s
								s += 7
							}
							return nil, errors.New("byte length overflows a 32-bit integer")
						}(r)
					if err != nil {
						return nil, fmt.Errorf("failed to read ready byte stream contents: %w", err)
					}
					slog.Debug("read ready byte stream contents", "len", len(buf))
					return wrpc.NewCompleteReader(bytes.NewReader(buf)), nil
				default:
					return nil, fmt.Errorf("invalid stream status byte %d", status)
				}
			}(r, append(path, 1)...)
			if err != nil {
				return nil, fmt.Errorf("failed to read `bytes` field: %w", err)
			}
			return v, nil
		}(r, []uint32{0}...)
		if err != nil {
			return fmt.Errorf("failed to read parameter 0: %w", err)
		}
		slog.DebugContext(ctx, "calling `wrpc-examples:streams/handler.echo` handler")
		r0, r1, err := h.Echo(ctx, p0)
		if err != nil {
			return fmt.Errorf("failed to handle `wrpc-examples:streams/handler.echo` invocation: %w", err)
		}

		var buf bytes.Buffer
		writes := make(map[uint32]func(wrpc.IndexWriter) error, 2)
		write0, err := func(v wrpc.ReceiveCompleter[[]uint64], w interface {
			io.ByteWriter
			io.Writer
		}) (write func(wrpc.IndexWriter) error, err error) {
			if v.IsComplete() {
				defer func() {
					body, ok := v.(io.Closer)
					if ok {
						if cErr := body.Close(); cErr != nil {
							if err == nil {
								err = fmt.Errorf("failed to close ready stream: %w", cErr)
							} else {
								slog.Warn("failed to close ready stream", "err", cErr)
							}
						}
					}
				}()
				slog.Debug("writing stream `stream::ready` status byte")
				if err = w.WriteByte(1); err != nil {
					return nil, fmt.Errorf("failed to write `stream::ready` byte: %w", err)
				}
				slog.Debug("receiving ready stream contents")
				vs, err := v.Receive()
				if err != nil && err != io.EOF {
					return nil, fmt.Errorf("failed to receive ready stream contents: %w", err)
				}
				if err != io.EOF && len(vs) > 0 {
					for {
						chunk, err := v.Receive()
						if err != nil && err != io.EOF {
							return nil, fmt.Errorf("failed to receive ready stream contents: %w", err)
						}
						if len(chunk) > 0 {
							vs = append(vs, chunk...)
						}
						if err == io.EOF {
							break
						}
					}
				}
				slog.Debug("writing ready stream contents", "len", len(vs))
				write, err := func(v []uint64, w interface {
					io.ByteWriter
					io.Writer
				}) (write func(wrpc.IndexWriter) error, err error) {
					n := len(v)
					if n > math.MaxUint32 {
						return nil, fmt.Errorf("list length of %d overflows a 32-bit integer", n)
					}
					if err = func(v int, w io.Writer) error {
						b := make([]byte, binary.MaxVarintLen32)
						i := binary.PutUvarint(b, uint64(v))
						slog.Debug("writing list length", "len", n)
						_, err = w.Write(b[:i])
						return err
					}(n, w); err != nil {
						return nil, fmt.Errorf("failed to write list length of %d: %w", n, err)
					}
					slog.Debug("writing list elements")
					writes := make(map[uint32]func(wrpc.IndexWriter) error, n)
					for i, e := range v {
						write, err := (func(wrpc.IndexWriter) error)(nil), func(v uint64, w io.Writer) (err error) {
							b := make([]byte, binary.MaxVarintLen64)
							i := binary.PutUvarint(b, uint64(v))
							slog.Debug("writing u64")
							_, err = w.Write(b[:i])
							return err
						}(e, w)
						if err != nil {
							return nil, fmt.Errorf("failed to write list element %d: %w", i, err)
						}
						if write != nil {
							writes[uint32(i)] = write
						}
					}
					if len(writes) > 0 {
						return func(w wrpc.IndexWriter) error {
							var wg sync.WaitGroup
							var wgErr atomic.Value
							for index, write := range writes {
								wg.Add(1)
								w, err := w.Index(index)
								if err != nil {
									return fmt.Errorf("failed to index writer: %w", err)
								}
								write := write
								go func() {
									defer wg.Done()
									if err := write(w); err != nil {
										wgErr.Store(err)
									}
								}()
							}
							wg.Wait()
							err := wgErr.Load()
							if err == nil {
								return nil
							}
							return err.(error)
						}, nil
					}
					return nil, nil
				}(vs, w)
				if err != nil {
					return nil, fmt.Errorf("failed to write ready stream contents: %w", err)
				}
				return write, nil
			} else {
				slog.Debug("writing stream `stream::pending` status byte")
				if err := w.WriteByte(0); err != nil {
					return nil, fmt.Errorf("failed to write `stream::pending` byte: %w", err)
				}
				return func(w wrpc.IndexWriter) (err error) {
					defer func() {
						body, ok := v.(io.Closer)
						if ok {
							if cErr := body.Close(); cErr != nil {
								if err == nil {
									err = fmt.Errorf("failed to close pending stream: %w", cErr)
								} else {
									slog.Warn("failed to close pending stream", "err", cErr)
								}
							}
						}
					}()
					var wg sync.WaitGroup
					var wgErr atomic.Value
					var total uint32
					for {
						var end bool
						slog.Debug("receiving outgoing pending stream contents")
						chunk, err := v.Receive()
						n := len(chunk)
						if n == 0 || err == io.EOF {
							end = true
							slog.Debug("outgoing pending stream reached EOF")
						} else if err != nil {
							return fmt.Errorf("failed to receive outgoing pending stream chunk: %w", err)
						}
						if n > math.MaxUint32 {
							return fmt.Errorf("outgoing pending stream chunk length of %d overflows a 32-bit integer", n)
						}
						if math.MaxUint32-uint32(n) < total {
							return errors.New("total outgoing pending stream element count would overflow a 32-bit unsigned integer")
						}
						slog.Debug("writing pending stream chunk length", "len", n)
						if err = wrpc.WriteUint32(uint32(n), w); err != nil {
							return fmt.Errorf("failed to write pending stream chunk length of %d: %w", n, err)
						}
						for _, v := range chunk {
							slog.Debug("writing pending stream element", "i", total)
							write, err := (func(wrpc.IndexWriter) error)(nil), func(v uint64, w io.Writer) (err error) {
								b := make([]byte, binary.MaxVarintLen64)
								i := binary.PutUvarint(b, uint64(v))
								slog.Debug("writing u64")
								_, err = w.Write(b[:i])
								return err
							}(v, w)
							if err != nil {
								return fmt.Errorf("failed to write pending stream chunk element %d: %w", total, err)
							}
							if write != nil {
								wg.Add(1)
								w, err := w.Index(total)
								if err != nil {
									return fmt.Errorf("failed to index writer: %w", err)
								}
								go func() {
									defer wg.Done()
									if err := write(w); err != nil {
										wgErr.Store(err)
									}
								}()
							}
							total++
						}
						if end {
							if err := w.WriteByte(0); err != nil {
								return fmt.Errorf("failed to write pending stream end byte: %w", err)
							}
							wg.Wait()
							err := wgErr.Load()
							if err == nil {
								return nil
							}
							return err.(error)
						}
					}
				}, nil
			}
		}(r0, &buf)
		if err != nil {
			return fmt.Errorf("failed to write result value 0: %w", err)
		}
		if write0 != nil {
			writes[0] = write0
		}
		write1, err := func(v wrpc.ReadCompleter, w interface {
			io.ByteWriter
			io.Writer
		}) (write func(wrpc.IndexWriter) error, err error) {
			if v.IsComplete() {
				defer func() {
					body, ok := v.(io.Closer)
					if ok {
						if cErr := body.Close(); cErr != nil {
							if err == nil {
								err = fmt.Errorf("failed to close ready byte stream: %w", cErr)
							} else {
								slog.Warn("failed to close ready byte stream", "err", cErr)
							}
						}
					}
				}()
				slog.Debug("writing byte stream `stream::ready` status byte")
				if err = w.WriteByte(1); err != nil {
					return nil, fmt.Errorf("failed to write `stream::ready` byte: %w", err)
				}
				slog.Debug("reading ready byte stream contents")
				var buf bytes.Buffer
				var n int64
				n, err = io.Copy(&buf, v)
				if err != nil {
					return nil, fmt.Errorf("failed to read ready byte stream contents: %w", err)
				}
				slog.Debug("writing ready byte stream contents", "len", n)
				if err = wrpc.WriteByteList(buf.Bytes(), w); err != nil {
					return nil, fmt.Errorf("failed to write ready byte stream contents: %w", err)
				}
				return nil, nil
			} else {
				slog.Debug("writing byte stream `stream::pending` status byte")
				if err = w.WriteByte(0); err != nil {
					return nil, fmt.Errorf("failed to write `stream::pending` byte: %w", err)
				}
				return func(w wrpc.IndexWriter) (err error) {
					defer func() {
						body, ok := v.(io.Closer)
						if ok {
							if cErr := body.Close(); cErr != nil {
								if err == nil {
									err = fmt.Errorf("failed to close pending byte stream: %w", cErr)
								} else {
									slog.Warn("failed to close pending byte stream", "err", cErr)
								}
							}
						}
					}()
					chunk := make([]byte, 8096)
					for {
						var end bool
						slog.Debug("reading pending byte stream contents")
						n, err := v.Read(chunk)
						if err == io.EOF {
							end = true
							slog.Debug("pending byte stream reached EOF")
						} else if err != nil {
							return fmt.Errorf("failed to read pending byte stream chunk: %w", err)
						}
						if n > math.MaxUint32 {
							return fmt.Errorf("pending byte stream chunk length of %d overflows a 32-bit integer", n)
						}
						slog.Debug("writing pending byte stream chunk length", "len", n)
						if err := wrpc.WriteUint32(uint32(n), w); err != nil {
							return fmt.Errorf("failed to write pending byte stream chunk length of %d: %w", n, err)
						}
						_, err = w.Write(chunk[:n])
						if err != nil {
							return fmt.Errorf("failed to write pending byte stream chunk contents: %w", err)
						}
						if end {
							if err := w.WriteByte(0); err != nil {
								return fmt.Errorf("failed to write pending byte stream end byte: %w", err)
							}
							return nil
						}
					}
				}, nil
			}
		}(r1, &buf)
		if err != nil {
			return fmt.Errorf("failed to write result value 1: %w", err)
		}
		if write1 != nil {
			writes[1] = write1
		}
		slog.DebugContext(ctx, "transmitting `wrpc-examples:streams/handler.echo` result")
		_, err = w.Write(buf.Bytes())
		if err != nil {
			return fmt.Errorf("failed to write result: %w", err)
		}
		if len(writes) > 0 {
			var wg sync.WaitGroup
			var wgErr atomic.Value
			for index, write := range writes {
				wg.Add(1)
				w, err := w.Index(index)
				if err != nil {
					return fmt.Errorf("failed to index writer: %w", err)
				}
				write := write
				go func() {
					defer wg.Done()
					if err := write(w); err != nil {
						wgErr.Store(err)
					}
				}()
			}
			wg.Wait()
			err := wgErr.Load()
			if err == nil {
				return nil
			}
			return err.(error)
		}
		return nil
	}, wrpc.NewSubscribePath().Index(0).Index(0), wrpc.NewSubscribePath().Index(0).Index(1))
	if err != nil {
		return nil, fmt.Errorf("failed to serve `wrpc-examples:streams/handler.echo`: %w", err)
	}
	stops = append(stops, stop0)
	return stop, nil
}