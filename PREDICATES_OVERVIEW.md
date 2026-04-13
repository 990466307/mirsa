## 1. 空指针 / 非空谓词

```rpl
is_null($ptr)
may_be_null($ptr)
```

理由：

- 现有 `is_null` 只能做简单传播，分支后或者取引用解引用后无法追踪。

## 2. 区间 / 边界谓词

```rpl
is_nonzero($x)
eq_const($x, $c)
lt_const($x, $c)
le_const($x, $c)
is_in_bounds($idx, $container)
```

理由：

- 目前没有数值相关的通用谓词。

## 3. 值关系谓词

```rpl
eq($x, $y)
lt($x, $y)
le($x, $y)
same_len($a, $b)
```

理由：

- 同上
